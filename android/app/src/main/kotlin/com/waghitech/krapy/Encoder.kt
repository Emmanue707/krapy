package com.waghitech.krapy

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.os.Build
import android.os.Bundle
import android.view.Surface
import java.nio.ByteBuffer
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

class Encoder(
    private val link: Link,
    private val captureBaseNs: Long,
) {
    private var codec: MediaCodec? = null
    private var drainThread: Thread? = null
    private val stop = AtomicBoolean(false)
    private var width: Int = 0
    private var height: Int = 0
    private var fps: Int = 30
    private var bitrate: Int = 4_000_000
    private var maxSize: Int = 1280
    private var frameId: Int = 0
    private val chunkBuf = ByteArray(Wire.MAX_VIDEO_DATA)
    private var codecConfig: ByteArray? = null
    private var needConfigBeforeKeyframe: Boolean = false
    var inputSurface: Surface? = null
        private set

    fun scaledSize(displayW: Int, displayH: Int, max: Int): Pair<Int, Int> {
        val limit = if (max > 0) max else maxSize
        if (displayW <= limit && displayH <= limit) {
            return align16(displayW) to align16(displayH)
        }
        val scale = min(limit.toDouble() / displayW, limit.toDouble() / displayH)
        var w = (displayW * scale).roundToInt()
        var h = (displayH * scale).roundToInt()
        w = align16(w)
        h = align16(h)
        w = max(16, w)
        h = max(16, h)
        return w to h
    }

    fun start(displayW: Int, displayH: Int, max: Int, fpsIn: Int, bitrateIn: Int) {
        stopDrain()
        releaseCodec()
        fps = fpsIn.coerceIn(1, 120)
        bitrate = bitrateIn.coerceAtLeast(100_000)
        maxSize = if (max > 0) max else maxSize
        val (w, h) = scaledSize(displayW, displayH, maxSize)
        width = w
        height = h
        codec = createCodec(w, h)
        inputSurface = codec?.createInputSurface()
            ?: throw IllegalStateException("encoder: no input surface")
        codec?.start()
        stop.set(false)
        drainThread = thread(name = "krapy-enc", isDaemon = true) { drainLoop() }
    }

    fun stop() {
        stop.set(true)
        stopDrain()
        releaseCodec()
        inputSurface = null
    }

    fun restart(newW: Int, newH: Int, surface: Surface?) {
        val oldMax = maxSize
        val oldFps = fps
        val oldBr = bitrate
        stop()
        if (surface != null) {
            start(newW, newH, oldMax, oldFps, oldBr)
            surface.let { s ->
                // surface replaced via createInputSurface; caller resets VirtualDisplay
            }
        } else {
            start(newW, newH, oldMax, oldFps, oldBr)
        }
        link.sendFormat(width, height)
    }

    fun setQuality(bitrateIn: Int, fpsIn: Int, max: Int) {
        if (max > 0) maxSize = max
        fps = fpsIn.coerceIn(1, 120)
        bitrate = bitrateIn.coerceAtLeast(100_000)
        val c = codec
        if (c != null) {
            val b = Bundle()
            b.putInt(MediaCodec.PARAMETER_KEY_VIDEO_BITRATE, bitrate)
            try {
                c.setParameters(b)
            } catch (_: Exception) {
            }
        }
    }

    fun restartCodecKeepingSurface() {
        val w = width
        val h = height
        if (w == 0 || h == 0) return
        stopDrain()
        releaseCodec()
        codec = createCodec(w, h)
        inputSurface = codec?.createInputSurface()
            ?: throw IllegalStateException("encoder: no input surface")
        codec?.start()
        stop.set(false)
        drainThread = thread(name = "krapy-enc", isDaemon = true) { drainLoop() }
    }

    fun requestKeyframe() {
        val c = codec ?: return
        val b = Bundle()
        b.putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0)
        try {
            c.setParameters(b)
        } catch (_: Exception) {
        }
    }

    fun applyMaxSize(displayW: Int, displayH: Int, max: Int) {
        maxSize = max
        val (nw, nh) = scaledSize(displayW, displayH, maxSize)
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
        drainThread = thread(name = "krapy-enc", isDaemon = true) { drainLoop() }
        fps = f
        bitrate = br
        link.sendFormat(width, height)
    }

    private fun createCodec(w: Int, h: Int): MediaCodec {
        var lastMsg = "unknown"
        try {
            return configureCodec(w, h, rich = true)
        } catch (e: Exception) {
            lastMsg = e.message ?: e.javaClass.simpleName
        }
        try {
            return configureCodec(w, h, rich = false)
        } catch (e: Exception) {
            lastMsg = e.message ?: e.javaClass.simpleName
        }
        val scale = min(min(720.0 / w, 720.0 / h), 1.0)
        val rw = align16(max(16, (w * scale).roundToInt()))
        val rh = align16(max(16, (h * scale).roundToInt()))
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
        format.setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 2)
        if (rich) {
            format.setInteger(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, 100_000)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
                format.setInteger(MediaFormat.KEY_PRIORITY, 0)
            }
            format.setInteger(MediaFormat.KEY_MAX_B_FRAMES, 0)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.LOLLIPOP) {
                try {
                    format.setInteger(
                        MediaFormat.KEY_BITRATE_MODE,
                        MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_CBR,
                    )
                } catch (_: Exception) {
                    format.setInteger(
                        MediaFormat.KEY_BITRATE_MODE,
                        MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_VBR,
                    )
                }
            }
            trySetProfile(format)
            trySetLowLatency(format)
        }
        val codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
        codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        return codec
    }

    private fun trySetProfile(format: MediaFormat) {
        try {
            format.setInteger(
                MediaFormat.KEY_PROFILE,
                MediaCodecInfo.CodecProfileLevel.AVCProfileConstrainedBaseline,
            )
            format.setInteger(MediaFormat.KEY_LEVEL, MediaCodecInfo.CodecProfileLevel.AVCLevel31)
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
        while (!stop.get()) {
            val idx = try {
                c.dequeueOutputBuffer(info, 5_000)
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
                        val cfg = annexB(csd0, csd1)
                        codecConfig = cfg
                        needConfigBeforeKeyframe = true
                    }
                }
                idx >= 0 -> {
                    val out = c.getOutputBuffer(idx) ?: continue
                    val size = info.size
                    if (size > 0) {
                        val flags = mediaFlags(info.flags)
                        val data = ByteArray(size)
                        out.position(info.offset)
                        out.limit(info.offset + size)
                        out.get(data)
                        val captureUs = captureUs(info.presentationTimeUs)
                        if (flags and Wire.VIDEO_FLAG_CODEC_CONFIG != 0) {
                            codecConfig = data
                            needConfigBeforeKeyframe = true
                        }
                        sendFrame(data, flags, captureUs)
                    }
                    c.releaseOutputBuffer(idx, false)
                }
            }
        }
    }

    private fun sendFrame(data: ByteArray, flags: Int, captureUs: Long) {
        var wireFlags = flags
        val cfg = codecConfig
        if (wireFlags and Wire.VIDEO_FLAG_KEYFRAME != 0 && cfg != null && needConfigBeforeKeyframe) {
            sendOneFrame(cfg, Wire.VIDEO_FLAG_CODEC_CONFIG, captureUs)
            needConfigBeforeKeyframe = false
        }
        sendOneFrame(data, wireFlags, captureUs)
    }

    private fun sendOneFrame(data: ByteArray, flags: Int, captureUs: Long) {
        val id = frameId
        val chunk = Wire.MAX_VIDEO_DATA
        val count = (data.size + chunk - 1) / chunk
        var off = 0
        var idx = 0
        while (off < data.size) {
            val len = min(chunk, data.size - off)
            System.arraycopy(data, off, chunkBuf, 0, len)
            link.sendVideo(id, idx, count, flags, captureUs, chunkBuf, 0, len)
            off += len
            idx++
        }
        frameId++
    }

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
        val deltaNs = presentationUs * 1000L
        return (captureBaseNs + deltaNs) / 1000L
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

    private fun align16(v: Int): Int = (v + 15) / 16 * 16

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