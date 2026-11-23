package com.waghitech.krapy

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.projection.MediaProjection
import android.os.Build
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.sqrt

class AudioCapture(
    private val projection: MediaProjection,
    private val link: Link,
    private val audioManager: AudioManager,
) {
    private val stop = AtomicBoolean(false)
    private var thread: Thread? = null
    private var record: AudioRecord? = null
    private val chunk = ByteArray(Wire.MAX_AUDIO_DATA)
    private var seq = 0
    private var savedMusicVolume: Int = -1
    private var savedMusicMuted: Boolean = false
    private var muteUnsupported = false

    fun start() {
        if (Build.VERSION.SDK_INT < 29) return
        stop.set(false)
        val config = AudioPlaybackCaptureConfiguration.Builder(projection)
            .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
            .addMatchingUsage(AudioAttributes.USAGE_GAME)
            .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
            .build()
        val format = AudioFormat.Builder()
            .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
            .setSampleRate(48000)
            .setChannelMask(AudioFormat.CHANNEL_IN_STEREO)
            .build()
        val minBuf = AudioRecord.getMinBufferSize(
            48000,
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
            return
        }
        record = rec
        savedMusicVolume = audioManager.getStreamVolume(AudioManager.STREAM_MUSIC)
        savedMusicMuted = audioManager.isStreamMute(AudioManager.STREAM_MUSIC)
        muteUnsupported = false
        link.sendAudioState(true)
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
        link.sendAudioState(false)
        restoreMusic()
    }

    private fun captureLoop(rec: AudioRecord) {
        val framesPerChunk = Wire.MAX_AUDIO_DATA / 4
        var ptsUs = System.nanoTime() / 1000
        var preMuteRms = 0.0
        var checkedMute = false
        while (!stop.get()) {
            val n = rec.read(chunk, 0, Wire.MAX_AUDIO_DATA)
            if (n < 0) break
            if (n == 0) continue
            val rms = rms16(chunk, n)
            if (!checkedMute && !muteUnsupported) {
                if (preMuteRms < 200.0 && rms >= 200.0) {
                    preMuteRms = rms
                    muteMusic()
                    checkedMute = true
                    continue
                }
            } else if (checkedMute && !muteUnsupported) {
                if (preMuteRms >= 200.0 && rms < 50.0) {
                    restoreMusic()
                    muteUnsupported = true
                }
                checkedMute = false
            }
            if (n >= Wire.MAX_AUDIO_DATA) {
                link.sendAudio(seq, ptsUs, chunk, 0, Wire.MAX_AUDIO_DATA)
                seq += 1
            }
            ptsUs += framesPerChunk * 1_000_000L / 48000
        }
    }

    private fun muteMusic() {
        audioManager.setStreamMute(AudioManager.STREAM_MUSIC, true)
    }

    private fun restoreMusic() {
        if (savedMusicVolume >= 0) {
            audioManager.setStreamMute(AudioManager.STREAM_MUSIC, savedMusicMuted)
            if (!savedMusicMuted) {
                audioManager.setStreamVolume(
                    AudioManager.STREAM_MUSIC,
                    savedMusicVolume,
                    0,
                )
            }
        }
    }

    private fun rms16(buf: ByteArray, len: Int): Double {
        var sum = 0.0
        var i = 0
        while (i + 1 < len) {
            val s = (buf[i].toInt() and 0xff) or (buf[i + 1].toInt() shl 8)
            val v = s.toShort().toInt()
            sum += (v * v).toDouble()
            i += 2
        }
        val n = len / 2
        return if (n == 0) 0.0 else sqrt(sum / n)
    }
}