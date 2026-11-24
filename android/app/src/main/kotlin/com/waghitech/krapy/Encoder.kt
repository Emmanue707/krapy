package com.waghitech.krapy

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.os.Build
import android.os.Bundle
import android.view.Surface
import java.nio.ByteBuffer
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min

class Encoder(
    private val link: Link,
) {
    private var codec: MediaCodec? = null
    private var drainThread: Thread? = null
    private val stop = AtomicBoolean(false)
    private var width: Int = 0
    private var height: Int = 0
    private var fps: Int = 60
    private var bitrate: Int = 16_000_000
    private var maxLong: Int = 2400
    private var displayW: Int = 0
    private var displayH: Int = 0
    private var frameId: Int = 0
    private val chunkBuf = ByteArray(Wire.MAX_TCP_VIDEO)
    private var drainBuf = ByteArray(64 * 1024)
    private var mergeBuf = ByteArray(64 * 1024)
    private var encAvgUs: Int = 0
    private var encMaxUs: Int = 0
    private var encWindowFrames: Int = 0
    private var encWindowSumUs: Long = 0
    private var encWindowMaxUs: Int = 0
    @Volatile
    private var codecConfig: ByteArray? = null
    @Volatile
    private var lastSyncRequestMs: Long = 0
    @Volatile
    private var useHighProfile: Boolean = true
    @Volatile
    private var pendingBaseline: Boolean = false
    private var lastOutPtsUs: Long = 0
    @Volatile
    private var dropUntilKeyframe: Boolean = false
    private val sendDrops = AtomicLong(0)
    private var nominalBitrate: Int = 16_000_000
    private var delayHighSinceMs: Long = 0
    private var delayLowSinceMs: Long = 0
    private var caps: MediaCodecInfo.VideoCapabilities? = null
    var inputSurface: Surface? = null
        private set

    fun scaledSize(displayWIn: Int, displayHIn: Int, max: Int): Pair<Int, Int> {
        val c = caps
        val maxL = if (max > 0) max else maxLong
        val (tw, th) = EncoderPlan.pickLongSideCap(displayWIn, displayHIn, maxL)
        if (c == null) {
            return alignLegacy(tw) to alignLegacy(th)
        }
        val wa = max(2, c.widthAlignment)
        val ha = max(2, c.heightAlignment)
        val sizeOk: (Int, Int) -> Boolean = { w, h ->
            c.areSizeAndRateSupported(w, h, fps.toDouble())
        }
        return EncoderPlan.pickSize(displayWIn, displayHIn, maxL, wa, ha, sizeOk)
    }

    fun start(displayWIn: Int, displayHIn: Int, max: Int, fpsIn: Int, bitrateIn: Int) {
        stopDrain()
        releaseCodec()
        displayW = displayWIn
        displayH = displayHIn
        fps = fpsIn.coerceIn(1, 120)
        bitrate = bitrateIn.coerceAtLeast(100_000)
        nominalBitrate = bitrate
        maxLong = if (max > 0) max else maxLong
        loadCaps()
        val (w, h) = scaledSize(displayW, displayH, maxLong)
        width = w
        height = h
        codec = createCodec(w, h)
        inputSurface = codec?.createInputSurface()
            ?: throw IllegalStateException("encoder: no input surface")
        codec?.start()
        stop.set(false)
        drainThread = Workers.start("krapy-enc") { drainLoop() }
    }

    fun stop() {
        stop.set(true)
        stopDrain()
        releaseCodec()
        inputSurface = null
        caps = null
    }

    fun restart(newW: Int, newH: Int, surface: Surface?) {
        val oldMax = maxLong
        val oldFps = fps
        val oldBr = bitrate
        displayW = newW
        displayH = newH
        stop()
        start(newW, newH, oldMax, oldFps, oldBr)
        link.sendFormat(width, height)
    }

    fun setQuality(bitrateIn: Int, fpsIn: Int, max: Int) {
        if (max > 0) maxLong = max
        fps = fpsIn.coerceIn(1, 120)
        bitrate = bitrateIn.coerceAtLeast(100_000)
        nominalBitrate = bitrate
        val c = codec
        if (c != null) {
            applyBitrate(c, bitrate)
        }
    }

    fun restartCodecKeepingSurface() {
        val w = width
        val h = height
        if (w == 0 || h == 0) return
        pendingBaseline = false
        lastOutPtsUs = 0
        stopDrain()
        releaseCodec()
        codec = createCodec(w, h)
        inputSurface = codec?.createInputSurface()
            ?: throw IllegalStateException("encoder: no input surface")
        codec?.start()
        stop.set(false)
        drainThread = Workers.start("krapy-enc") { drainLoop() }
    }

    fun requestKeyframe() {
        val now = System.currentTimeMillis()
        if (now - lastSyncRequestMs < 300) return
        lastSyncRequestMs = now
        val c = codec ?: return
        val b = Bundle()
        b.putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0)
        try {
            c.setParameters(b)
        } catch (_: Exception) {
        }
    }

    fun applyMaxSize(displayWIn: Int, displayHIn: Int, max: Int) {
        displayW = displayWIn
        displayH = displayHIn
        maxLong = max
        val (nw, nh) = scaledSize(displayW, displayH, maxLong)
        if (nw == width && nh == height) return
        val br = bitrate
        val f = fps
        stopDrain()
        releaseCodec()
        width = nw
        height = nh
        codec = createCodec(nw, nh)
        inputSurface = codec?.createInputSurface()
            ?: throw IllegalStateException("encoder: no input surface")
        codec?.start()
        stop.set(false)
        drainThread = Workers.start("krapy-enc") { drainLoop() }
        fps = f
        bitrate = br
        nominalBitrate = br
        link.sendFormat(width, height)
    }

    private fun loadCaps() {
        caps = try {
            val c = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
            val info = c.codecInfo.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_AVC)
            c.release()
            info.videoCapabilities
        } catch (_: Exception) {
            null
        }
    }

    private fun createCodec(w: Int, h: Int): MediaCodec {
        var lastMsg = "unknown"
        try {
            return configureCodec(w, h, rich = true)
        } catch (e: Exception) {
            lastMsg = e.message ?: e.javaClass.simpleName
            if (useHighProfile) {
                useHighProfile = false
                try {
                    return configureCodec(w, h, rich = true)
                } catch (_: Exception) {
                }
            }
        }
        try {
            return configureCodec(w, h, rich = false)
        } catch (e: Exception) {
            lastMsg = e.message ?: e.javaClass.simpleName
        }
        val scale = min(min(720.0 / w, 720.0 / h), 1.0)
        val rw = alignLegacy(max(16, (w * scale).toInt()))
        val rh = alignLegacy(max(16, (h * scale).toInt()))
        try {
            return configureCodec(rw, rh, rich = false)
        } catch (e: Exception) {
            throw IllegalStateException("encoder: ${e.message ?: lastMsg}")
        }
    }

    private fun configureCodec(w: Int, h: Int, rich: Boolean): MediaCodec {
        val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, w, h)
        format.setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
        format.setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
        format.setInteger(MediaFormat.KEY_FRAME_RATE, fps)
        format.setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 10)
        if (rich) {
            format.setInteger(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, 100_000)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
                try {
                    format.setInteger(MediaFormat.KEY_OPERATING_RATE, fps)
                } catch (_: Exception) {
                }
                try {
                    format.setInteger(MediaFormat.KEY_PRIORITY, 0)
                } catch (_: Exception) {
                }
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                try {
                    format.setFloat(MediaFormat.KEY_MAX_FPS_TO_ENCODER, fps.toFloat())
                } catch (_: Exception) {
                }
            }
            format.setInteger(MediaFormat.KEY_MAX_B_FRAMES, 0)
            format.setInteger(MediaFormat.KEY_PREPEND_HEADER_TO_SYNC_FRAMES, 1)
            setBitrateMode(format)
            trySetProfile(format, w, h)
            trySetLowLatency(format)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
                try {
                    format.setInteger(MediaFormat.KEY_COLOR_RANGE, MediaFormat.COLOR_RANGE_LIMITED)
                    format.setInteger(MediaFormat.KEY_COLOR_STANDARD, MediaFormat.COLOR_STANDARD_BT709)
                    format.setInteger(MediaFormat.KEY_COLOR_TRANSFER, MediaFormat.COLOR_TRANSFER_SDR_VIDEO)
                } catch (_: Exception) {
                }
            }
        }
        val codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
        codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        return codec
    }

    private fun setBitrateMode(format: MediaFormat) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.LOLLIPOP) return
        val modeCbFd = 3
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            try {
                format.setInteger(MediaFormat.KEY_BITRATE_MODE, modeCbFd)
                return
            } catch (_: Exception) {
            }
        }
        try {
            format.setInteger(
                MediaFormat.KEY_BITRATE_MODE,
                MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_CBR,
            )
        } catch (_: Exception) {
            try {
                format.setInteger(
                    MediaFormat.KEY_BITRATE_MODE,
                    MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_VBR,
                )
            } catch (_: Exception) {
            }
        }
    }

    private fun trySetProfile(format: MediaFormat, w: Int, h: Int) {
        val level = EncoderPlan.pickLevel(w, h, fps)
        if (useHighProfile) {
            try {
                format.setInteger(MediaFormat.KEY_PROFILE, MediaCodecInfo.CodecProfileLevel.AVCProfileHigh)
                if (level != null) format.setInteger(MediaFormat.KEY_LEVEL, level)
                return
            } catch (_: Exception) {
            }
        }
        try {
            format.setInteger(
                MediaFormat.KEY_PROFILE,
                MediaCodecInfo.CodecProfileLevel.AVCProfileConstrainedBaseline,
            )
            if (level != null) format.setInteger(MediaFormat.KEY_LEVEL, level)
        } catch (_: Exception) {
            try {
                format.setInteger(MediaFormat.KEY_PROFILE, MediaCodecInfo.CodecProfileLevel.AVCProfileBaseline)
            } catch (_: Exception) {
            }
        }
    }

    private fun trySetLowLatency(format: MediaFormat) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            try {
                format.setInteger(MediaFormat.KEY_LATENCY, 1)
            } catch (_: Exception) {
            }
            try {
                format.setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
            } catch (_: Exception) {
            }
        }
    }

    private fun drainLoop() {
        val info = MediaCodec.BufferInfo()
        val c = codec ?: return
        val frameTimeUs = 1_000_000L / fps.coerceAtLeast(1)
        while (!stop.get()) {
            val idx = try {
                c.dequeueOutputBuffer(info, 10_000)
            } catch (_: Exception) {
                break
            }
            when {
                idx == MediaCodec.INFO_TRY_AGAIN_LATER -> continue
                idx == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                    val newFmt = c.outputFormat
                    val csd0 = newFmt.getByteBuffer("csd-0")
                    val csd1 = newFmt.getByteBuffer("csd-1")
                    if (csd0 != null) {
                        codecConfig = annexB(csd0, csd1)
                    }
                }
                idx >= 0 -> {
                    val out = c.getOutputBuffer(idx) ?: continue
                    val size = info.size
                    if (size > 0) {
                        val flags = mediaFlags(info.flags)
                        if (drainBuf.size < size) drainBuf = ByteArray(size + size / 2)
                        val data = drainBuf
                        out.position(info.offset)
                        out.limit(info.offset + size)
                        out.get(data, 0, size)
                        checkOutputOrder(info.presentationTimeUs)
                        val captureUs = captureUs(info.presentationTimeUs)
                        val encUs = ((System.nanoTime() / 1000) - captureUs).toInt().coerceAtLeast(0)
                        tickEncodeDelay(encUs, frameTimeUs)
                        if (flags and Wire.VIDEO_FLAG_CODEC_CONFIG != 0) {
                            codecConfig = data.copyOf(size)
                        } else {
                            sendFrame(data, size, flags, captureUs)
                        }
                    }
                    c.releaseOutputBuffer(idx, false)
                }
            }
        }
    }

    private fun tickEncodeDelay(encUs: Int, frameTimeUs: Long) {
        encWindowFrames++
        encWindowSumUs += encUs
        if (encUs > encWindowMaxUs) encWindowMaxUs = encUs
        val now = System.currentTimeMillis()
        val late = encUs > frameTimeUs * 2
        if (late) {
            if (delayHighSinceMs == 0L) delayHighSinceMs = now
            delayLowSinceMs = 0
            if (now - delayHighSinceMs >= 1000) {
                val next = (bitrate * 0.85).toInt().coerceAtLeast(nominalBitrate / 2)
                if (next < bitrate) {
                    bitrate = next
                    codec?.let { applyBitrate(it, bitrate) }
                }
                delayHighSinceMs = now
            }
        } else {
            delayHighSinceMs = 0
            if (delayLowSinceMs == 0L) delayLowSinceMs = now
            if (now - delayLowSinceMs >= 1000 && bitrate < nominalBitrate) {
                val next = min(nominalBitrate, (bitrate * 1.1).toInt())
                if (next > bitrate) {
                    bitrate = next
                    codec?.let { applyBitrate(it, bitrate) }
                }
                delayLowSinceMs = now
            }
        }
    }

    private fun applyBitrate(c: MediaCodec, br: Int) {
        val b = Bundle()
        b.putInt(MediaCodec.PARAMETER_KEY_VIDEO_BITRATE, br)
        try {
            c.setParameters(b)
        } catch (_: Exception) {
        }
    }

    private fun checkOutputOrder(ptsUs: Long) {
        if (useHighProfile && lastOutPtsUs > 0 && ptsUs < lastOutPtsUs) {
            // frames came out of order: the encoder reorders in high profile
            useHighProfile = false
            pendingBaseline = true
        }
        lastOutPtsUs = ptsUs
    }

    private fun sendFrame(data: ByteArray, len: Int, flags: Int, captureUs: Long) {
        val ageUs = System.nanoTime() / 1000 - captureUs
        if (VideoSendGate.tooOld(flags, ageUs) && !dropUntilKeyframe) {
            dropUntilKeyframe = true
            requestKeyframe()
        }
        if (!VideoSendGate.shouldSend(flags, dropUntilKeyframe)) {
            sendDrops.incrementAndGet()
            return
        }
        val cfg = codecConfig
        if (flags and Wire.VIDEO_FLAG_KEYFRAME != 0 && cfg != null && !Annex.hasSps(data, len)) {
            val total = cfg.size + len
            if (mergeBuf.size < total) mergeBuf = ByteArray(total + total / 2)
            System.arraycopy(cfg, 0, mergeBuf, 0, cfg.size)
            System.arraycopy(data, 0, mergeBuf, cfg.size, len)
            sendOneFrame(mergeBuf, total, flags, captureUs)
        } else {
            sendOneFrame(data, len, flags, captureUs)
        }
        dropUntilKeyframe = VideoSendGate.onSent(flags, dropUntilKeyframe)
    }

    private fun sendOneFrame(data: ByteArray, len: Int, flags: Int, captureUs: Long) {
        val id = frameId
        val chunk = link.maxChunk()
        val count = (len + chunk - 1) / chunk
        var off = 0
        var idx = 0
        while (off < len) {
            val part = min(chunk, len - off)
            System.arraycopy(data, off, chunkBuf, 0, part)
            link.sendVideo(id, idx, count, flags, captureUs, chunkBuf, 0, part)
            off += part
            idx++
        }
        frameId++
    }

    fun drainEncoderStatsWindow() {
        if (encWindowFrames > 0) {
            encAvgUs = (encWindowSumUs / encWindowFrames).toInt()
            encMaxUs = encWindowMaxUs
        }
        encWindowFrames = 0
        encWindowSumUs = 0
        encWindowMaxUs = 0
    }

    fun encoderStats(): Triple<Int, Int, Int> = Triple(encAvgUs, encMaxUs, fps)

    fun sendDropCount(): Long = sendDrops.get()

    fun needsBaselineRestart(): Boolean = pendingBaseline

    private fun mediaFlags(codecFlags: Int): Int {
        var f = 0
        if (codecFlags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) {
            f = f or Wire.VIDEO_FLAG_CODEC_CONFIG
        }
        if (codecFlags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0) {
            f = f or Wire.VIDEO_FLAG_KEYFRAME
        }
        return f
    }

    private fun captureUs(presentationUs: Long): Long {
        val nowUs = System.nanoTime() / 1000
        return if (abs(nowUs - presentationUs) < 5_000_000) presentationUs else nowUs
    }

    private fun annexB(csd0: ByteBuffer, csd1: ByteBuffer?): ByteArray {
        val a = byteArrayOf(0, 0, 0, 1)
        val sps = ByteArray(csd0.remaining())
        csd0.get(sps)
        if (csd1 == null) return a + sps
        val pps = ByteArray(csd1.remaining())
        csd1.get(pps)
        return a + sps + a + pps
    }

    private fun alignLegacy(v: Int): Int = (v + 15) / 16 * 16

    private fun stopDrain() {
        drainThread?.interrupt()
        drainThread = null
    }

    private fun releaseCodec() {
        try {
            codec?.stop()
        } catch (_: Exception) {
        }
        try {
            codec?.release()
        } catch (_: Exception) {
        }
        codec = null
    }

    fun currentWidth(): Int = width
    fun currentHeight(): Int = height
    fun currentFps(): Int = fps
    fun currentBitrate(): Int = bitrate
}