package com.waghitech.krapy

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.projection.MediaProjection
import android.os.Build
import android.os.SystemClock
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.abs
import kotlin.math.min

object AudioChunker {
    fun feed(
        read: ByteArray,
        readLen: Int,
        carry: ByteArray,
        carryLen: Int,
        chunkSize: Int,
        emit: (ByteArray) -> Unit,
    ): Int {
        var off = 0
        var clen = carryLen
        while (off < readLen) {
            if (clen < chunkSize) {
                val need = chunkSize - clen
                val take = min(need, readLen - off)
                System.arraycopy(read, off, carry, clen, take)
                clen += take
                off += take
            }
            if (clen >= chunkSize) {
                emit(carry)
                clen = 0
            }
        }
        return clen
    }
}

object AudioMuteCheck {
    const val PEAK_SILENT = 8
    const val WAIT_MS = 1500L

    fun captureAlsoSilenced(musicActive: Boolean, maxPeak: Int, elapsedMs: Long): Boolean =
        musicActive && maxPeak < PEAK_SILENT && elapsedMs >= WAIT_MS
}

class AudioCapture(
    private val projection: MediaProjection,
    private val link: Link,
    private val audioManager: AudioManager,
    private val ctx: Context,
    private val onMuteFailed: () -> Unit,
) {
    private val stop = AtomicBoolean(false)
    private var thread: Thread? = null
    private var record: AudioRecord? = null
    private var sampleRate = 48000
    private var channels = 2
    private val readBuf = ByteArray(Wire.MAX_AUDIO_DATA)
    private val carry = ByteArray(Wire.MAX_AUDIO_DATA)
    private var carryLen = 0
    private var seq = 0
    private var framePts = 0L
    private var muteChecked = false
    private var maxPeakSinceMute = 0
    private var muteStartedMs = 0L
    private var musicSinceMs = 0L

    fun start() {
        if (Build.VERSION.SDK_INT < 29) return
        stop.set(false)
        val rec = openRecord(48000) ?: openRecord(44100) ?: return
        sampleRate = rec.getSampleRate()
        channels = 2
        record = rec
        framePts = System.nanoTime() / 1000
        muteForPc()
        link.sendAudioState(true, sampleRate, channels, phoneMuted = true)
        rec.startRecording()
        thread = Workers.start("krapy-audio") { captureLoop(rec) }
    }

    fun stop() {
        stop.set(true)
        thread?.interrupt()
        thread = null
        try {
            record?.stop()
        } catch (_: Exception) {
        }
        try {
            record?.release()
        } catch (_: Exception) {
        }
        record = null
        carryLen = 0
        link.sendAudioState(false, sampleRate, channels, phoneMuted = true)
        restoreMusic()
    }

    private fun openRecord(rate: Int): AudioRecord? {
        val config = AudioPlaybackCaptureConfiguration.Builder(projection)
            .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
            .addMatchingUsage(AudioAttributes.USAGE_GAME)
            .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
            .build()
        val format = AudioFormat.Builder()
            .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
            .setSampleRate(rate)
            .setChannelMask(AudioFormat.CHANNEL_IN_STEREO)
            .build()
        val minBuf = AudioRecord.getMinBufferSize(
            rate,
            AudioFormat.CHANNEL_IN_STEREO,
            AudioFormat.ENCODING_PCM_16BIT,
        )
        val rec = AudioRecord.Builder()
            .setAudioFormat(format)
            .setBufferSizeInBytes(minBuf.coerceAtLeast(Wire.MAX_AUDIO_DATA * 4))
            .setAudioPlaybackCaptureConfig(config)
            .build()
        if (rec.state != AudioRecord.STATE_INITIALIZED) {
            rec.release()
            return null
        }
        return rec
    }

    private fun muteForPc() {
        val vol = audioManager.getStreamVolume(AudioManager.STREAM_MUSIC)
        SessionPrefs.setSavedMusicVolume(ctx, vol)
        audioManager.setStreamVolume(AudioManager.STREAM_MUSIC, 0, 0)
        muteStartedMs = SystemClock.elapsedRealtime()
        maxPeakSinceMute = 0
        musicSinceMs = 0L
        muteChecked = false
    }

    private fun restoreMusic() {
        val saved = SessionPrefs.takeSavedMusicVolume(ctx) ?: return
        audioManager.setStreamVolume(AudioManager.STREAM_MUSIC, saved, 0)
    }

    private fun captureLoop(rec: AudioRecord) {
        val framesPerChunk = Wire.MAX_AUDIO_DATA / (channels * 2)
        while (!stop.get()) {
            val n = rec.read(readBuf, 0, Wire.MAX_AUDIO_DATA)
            if (n < 0) break
            if (n == 0) continue
            val peak = peak16(readBuf, n)
            if (peak > maxPeakSinceMute) maxPeakSinceMute = peak
            if (!muteChecked) {
                val now = SystemClock.elapsedRealtime()
                if (audioManager.isMusicActive) {
                    if (musicSinceMs == 0L) musicSinceMs = now
                } else {
                    musicSinceMs = 0L
                    maxPeakSinceMute = 0
                }
                if (musicSinceMs != 0L) {
                    if (AudioMuteCheck.captureAlsoSilenced(true, maxPeakSinceMute, now - musicSinceMs)) {
                        muteChecked = true
                        restoreMusic()
                        link.sendAudioState(true, sampleRate, channels, phoneMuted = false)
                        onMuteFailed()
                    } else if (maxPeakSinceMute >= AudioMuteCheck.PEAK_SILENT) {
                        muteChecked = true
                    }
                }
            }
            carryLen = AudioChunker.feed(readBuf, n, carry, carryLen, Wire.MAX_AUDIO_DATA) { chunk ->
                link.sendAudio(seq, framePts, chunk, 0, Wire.MAX_AUDIO_DATA)
                seq += 1
                framePts += framesPerChunk * 1_000_000L / sampleRate
            }
        }
    }

    private fun peak16(buf: ByteArray, len: Int): Int {
        var peak = 0
        var i = 0
        while (i + 1 < len) {
            val s = (buf[i].toInt() and 0xff) or (buf[i + 1].toInt() shl 8)
            val v = abs(s.toShort().toInt())
            if (v > peak) peak = v
            i += 2
        }
        return peak
    }
}