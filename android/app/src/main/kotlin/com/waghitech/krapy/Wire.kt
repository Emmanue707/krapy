package com.waghitech.krapy

import java.nio.ByteBuffer
import java.nio.ByteOrder

object Wire {
    const val DG_VIDEO: Int = 0x01
    const val DG_PING: Int = 0x02
    const val DG_PONG: Int = 0x03
    const val DG_KEEPALIVE: Int = 0x05
    const val DG_AUDIO: Int = 0x06

    const val VIDEO_FLAG_KEYFRAME: Int = 1 shl 0
    const val VIDEO_FLAG_CODEC_CONFIG: Int = 1 shl 1

    const val MAX_VIDEO_DATA: Int = 1100
    const val MAX_TCP_VIDEO: Int = 32 * 1024
    const val MAX_AUDIO_DATA: Int = 960

    fun encodeTcpFrame(body: ByteArray): ByteArray {
        val out = ByteArray(2 + body.size)
        out[0] = (body.size shr 8).toByte()
        out[1] = body.size.toByte()
        System.arraycopy(body, 0, out, 2, body.size)
        return out
    }

    fun readTcpFrameLength(header: ByteArray): Int {
        if (header.size < 2) return -1
        return ((header[0].toInt() and 0xff) shl 8) or (header[1].toInt() and 0xff)
    }

    fun encodeHello(phoneId: ByteArray, nonceP: ByteArray, name: String): ByteArray {
        val nameBytes = name.toByteArray(Charsets.UTF_8)
        val out = ByteArray(4 + 1 + 8 + 16 + 1 + nameBytes.size)
        out[0] = 'K'.code.toByte()
        out[1] = 'R'.code.toByte()
        out[2] = 'P'.code.toByte()
        out[3] = 'Y'.code.toByte()
        out[4] = 1
        System.arraycopy(phoneId, 0, out, 5, 8)
        System.arraycopy(nonceP, 0, out, 13, 16)
        out[29] = nameBytes.size.toByte()
        System.arraycopy(nameBytes, 0, out, 30, nameBytes.size)
        return out
    }

    fun encodeWelcome(desktopId: ByteArray, nonceD: ByteArray): ByteArray {
        val out = ByteArray(29)
        out[0] = 'K'.code.toByte()
        out[1] = 'R'.code.toByte()
        out[2] = 'P'.code.toByte()
        out[3] = 'Y'.code.toByte()
        out[4] = 1
        System.arraycopy(desktopId, 0, out, 5, 8)
        System.arraycopy(nonceD, 0, out, 13, 16)
        return out
    }

    fun decodeWelcome(body: ByteArray): Pair<ByteArray, ByteArray> {
        if (body.size < 29) throw WireException("truncated welcome")
        if (body[0] != 'K'.code.toByte() || body[1] != 'R'.code.toByte() ||
            body[2] != 'P'.code.toByte() || body[3] != 'Y'.code.toByte()
        ) {
            throw WireException("bad magic")
        }
        if (body[4] != 1.toByte()) throw WireException("bad version")
        val desktopId = body.copyOfRange(5, 13)
        val nonceD = body.copyOfRange(13, 29)
        return desktopId to nonceD
    }

    sealed class Control {
        data class Start(
            val width: Int,
            val height: Int,
            val fps: Int,
            val bitrate: Int,
            val codec: Int,
        ) : Control()

        data class Quality(val bitrate: Int, val fps: Int, val maxSize: Int) : Control()
        data object Keyframe : Control()
        data class Format(val width: Int, val height: Int) : Control()
        data class Clipboard(val text: String) : Control()
        data class Touch(val action: Int, val pointer: Int, val x: Int, val y: Int) : Control()
        data class Key(val code: Int) : Control()
        data class Text(val text: String) : Control()
        data class DesktopStats(
            val fps: Int,
            val latencyMs: Int,
            val decodeMs: Int,
            val lossPermille: Int,
        ) : Control()
        data class SetSetting(val key: Int, val value: Int) : Control()
        data object Bye : Control()
    }

    // video over tcp (control 0x30), same header as the udp video datagram
    fun encodeVideoChunk(
        frameId: Int,
        idx: Int,
        count: Int,
        flags: Int,
        captureUs: Long,
        data: ByteArray,
        off: Int,
        len: Int,
    ): ByteArray {
        require(len <= MAX_TCP_VIDEO)
        val b = ByteBuffer.allocate(18 + len).order(ByteOrder.BIG_ENDIAN)
        b.put(0x30.toByte())
        b.putInt(frameId)
        b.putShort(idx.toShort())
        b.putShort(count.toShort())
        b.put(flags.toByte())
        b.putLong(captureUs)
        b.put(data, off, len)
        return b.array()
    }

    fun encodeControl(c: Control): ByteArray = when (c) {
        is Control.Start -> {
            val out = ByteArray(11)
            out[0] = 0x01
            putU16(out, 1, c.width)
            putU16(out, 3, c.height)
            out[5] = c.fps.toByte()
            putU32(out, 6, c.bitrate)
            out[10] = c.codec.toByte()
            out
        }

        is Control.Format -> {
            val out = ByteArray(5)
            out[0] = 0x04
            putU16(out, 1, c.width)
            putU16(out, 3, c.height)
            out
        }

        is Control.Clipboard -> {
            val t = c.text.toByteArray(Charsets.UTF_8)
            val out = ByteArray(1 + t.size)
            out[0] = 0x10
            System.arraycopy(t, 0, out, 1, t.size)
            out
        }

        else -> throw WireException("phone cannot encode ${c::class.simpleName}")
    }

    fun decodeControl(buf: ByteArray): Control {
        if (buf.isEmpty()) throw WireException("empty control")
        return when (buf[0].toInt() and 0xff) {
            0x01 -> {
                if (buf.size < 11) throw WireException("truncated start")
                Control.Start(
                    width = u16(buf, 1),
                    height = u16(buf, 3),
                    fps = buf[5].toInt() and 0xff,
                    bitrate = u32(buf, 6),
                    codec = buf[10].toInt() and 0xff,
                )
            }

            0x02 -> {
                if (buf.size < 8) throw WireException("truncated quality")
                Control.Quality(
                    bitrate = u32(buf, 1),
                    fps = buf[5].toInt() and 0xff,
                    maxSize = u16(buf, 6),
                )
            }

            0x03 -> Control.Keyframe
            0x10 -> Control.Clipboard(String(buf, 1, buf.size - 1, Charsets.UTF_8))
            0x20 -> {
                if (buf.size < 7) throw WireException("truncated touch")
                Control.Touch(
                    action = buf[1].toInt() and 0xff,
                    pointer = buf[2].toInt() and 0xff,
                    x = u16(buf, 3),
                    y = u16(buf, 5),
                )
            }

            0x21 -> {
                if (buf.size < 2) throw WireException("truncated key")
                Control.Key(buf[1].toInt() and 0xff)
            }

            0x22 -> Control.Text(String(buf, 1, buf.size - 1, Charsets.UTF_8))
            0x41 -> {
                if (buf.size < 8) throw WireException("truncated desktop stats")
                Control.DesktopStats(
                    fps = buf[1].toInt() and 0xff,
                    latencyMs = u16(buf, 2),
                    decodeMs = u16(buf, 4),
                    lossPermille = u16(buf, 6),
                )
            }
            0x47 -> {
                if (buf.size < 3) throw WireException("truncated set setting")
                Control.SetSetting(
                    key = buf[1].toInt() and 0xff,
                    value = buf[2].toInt() and 0xff,
                )
            }
            0x7f -> Control.Bye
            else -> throw WireException("unknown control ${buf[0]}")
        }
    }

    fun decodePhoneStats(buf: ByteArray): List<Int> {
        if (buf.size < 14) throw WireException("truncated phone stats")
        return listOf(
            u32(buf, 1),
            u32(buf, 5),
            buf[9].toInt() and 0xff,
            u32(buf, 10),
        )
    }

    fun encodePhoneStats(encAvgUs: Int, encMaxUs: Int, fps: Int, bitrate: Int): ByteArray {
        val out = ByteArray(14)
        out[0] = 0x40
        putU32(out, 1, encAvgUs)
        putU32(out, 5, encMaxUs)
        out[9] = fps.toByte()
        putU32(out, 10, bitrate)
        return out
    }

    fun encodePreset(id: Int): ByteArray = byteArrayOf(0x42, id.toByte())

    fun encodePaused(on: Boolean): ByteArray = byteArrayOf(0x43, if (on) 1 else 0)

    fun encodeRotate(): ByteArray = byteArrayOf(0x45)

    fun encodeAudioState(on: Boolean, rate: Int, channels: Int, phoneMuted: Boolean): ByteArray {
        val out = ByteArray(8)
        out[0] = 0x46
        out[1] = if (on) 1 else 0
        putU32Le(out, 2, rate)
        out[6] = channels.toByte()
        out[7] = if (phoneMuted) 1 else 0
        return out
    }

    fun encodeSettings(
        paused: Boolean,
        preset: Int,
        soundPc: Boolean,
        keepAwake: Boolean,
        clipboard: Boolean,
        rotation: Int,
    ): ByteArray = byteArrayOf(
        0x49,
        if (paused) 1 else 0,
        preset.toByte(),
        if (soundPc) 1 else 0,
        if (keepAwake) 1 else 0,
        if (clipboard) 1 else 0,
        (rotation and 3).toByte(),
    )

    fun encodeDesktopStats(fps: Int, latencyMs: Int, decodeMs: Int, lossPermille: Int): ByteArray {
        val out = ByteArray(8)
        out[0] = 0x41
        out[1] = fps.toByte()
        putU16(out, 2, latencyMs)
        putU16(out, 4, decodeMs)
        putU16(out, 6, lossPermille)
        return out
    }

    fun encodeAudioChunk(seq: Int, ptsUs: Long, data: ByteArray, off: Int, len: Int): ByteArray {
        require(len <= MAX_AUDIO_DATA)
        val b = ByteBuffer.allocate(13 + len).order(ByteOrder.BIG_ENDIAN)
        b.put(0x31.toByte())
        b.putInt(seq)
        b.putLong(ptsUs)
        b.put(data, off, len)
        return b.array()
    }

    fun encodeAudioDatagram(seq: Int, ptsUs: Long, data: ByteArray, off: Int, len: Int): ByteArray {
        require(len <= MAX_AUDIO_DATA)
        val out = ByteArray(13 + len)
        out[0] = DG_AUDIO.toByte()
        putU32(out, 1, seq)
        putU64(out, 5, ptsUs)
        System.arraycopy(data, off, out, 13, len)
        return out
    }

    fun encodeDatagramBody(
        type: Int,
        frameId: Int = 0,
        idx: Int = 0,
        count: Int = 0,
        flags: Int = 0,
        captureUs: Long = 0,
        data: ByteArray = ByteArray(0),
        pingId: Int = 0,
        pingT: Long = 0,
        phoneUs: Long = 0,
    ): ByteArray = when (type) {
        DG_VIDEO -> {
            val out = ByteArray(18 + data.size)
            out[0] = type.toByte()
            putU32(out, 1, frameId)
            putU16(out, 5, idx)
            putU16(out, 7, count)
            out[9] = flags.toByte()
            putU64(out, 10, captureUs)
            System.arraycopy(data, 0, out, 18, data.size)
            out
        }

        DG_PONG -> {
            val out = if (phoneUs != 0L) ByteArray(21) else ByteArray(13)
            out[0] = type.toByte()
            putU32(out, 1, pingId)
            putU64(out, 5, pingT)
            if (phoneUs != 0L) putU64(out, 13, phoneUs)
            out
        }

        DG_KEEPALIVE -> byteArrayOf(type.toByte())

        else -> throw WireException("bad datagram type")
    }

    fun decodeDatagramPlain(buf: ByteArray): DatagramPlain {
        if (buf.isEmpty()) throw WireException("empty datagram")
        return when (buf[0].toInt() and 0xff) {
            DG_PING -> {
                if (buf.size < 13) throw WireException("truncated ping")
                DatagramPlain.Ping(u32(buf, 1), u64(buf, 5))
            }

            DG_PONG, DG_VIDEO, DG_KEEPALIVE -> throw WireException("unexpected dg type on receive")
            else -> throw WireException("unknown dg ${buf[0]}")
        }
    }

    sealed class DatagramPlain {
        data class Ping(val id: Int, val tUs: Long) : DatagramPlain()
    }

    fun hexId(bytes: ByteArray): String = bytes.joinToString("") { "%02x".format(it) }

    fun hexToBytes(hex: String): ByteArray {
        require(hex.length == 16) { "id must be 16 hex" }
        return ByteArray(8) { i -> hex.substring(i * 2, i * 2 + 2).toInt(16).toByte() }
    }

    private fun putU16(out: ByteArray, off: Int, v: Int) {
        out[off] = (v shr 8).toByte()
        out[off + 1] = v.toByte()
    }

    private fun putU32(out: ByteArray, off: Int, v: Int) {
        out[off] = (v shr 24).toByte()
        out[off + 1] = (v shr 16).toByte()
        out[off + 2] = (v shr 8).toByte()
        out[off + 3] = v.toByte()
    }

    private fun putU32Le(out: ByteArray, off: Int, v: Int) {
        out[off] = v.toByte()
        out[off + 1] = (v shr 8).toByte()
        out[off + 2] = (v shr 16).toByte()
        out[off + 3] = (v shr 24).toByte()
    }

    private fun putU64(out: ByteArray, off: Int, v: Long) {
        ByteBuffer.wrap(out, off, 8).order(ByteOrder.BIG_ENDIAN).putLong(v)
    }

    private fun u16(buf: ByteArray, off: Int): Int =
        ((buf[off].toInt() and 0xff) shl 8) or (buf[off + 1].toInt() and 0xff)

    private fun u32(buf: ByteArray, off: Int): Int =
        ((buf[off].toInt() and 0xff) shl 24) or
            ((buf[off + 1].toInt() and 0xff) shl 16) or
            ((buf[off + 2].toInt() and 0xff) shl 8) or
            (buf[off + 3].toInt() and 0xff)

    private fun u64(buf: ByteArray, off: Int): Long =
        ByteBuffer.wrap(buf, off, 8).order(ByteOrder.BIG_ENDIAN).long
}

class WireException(message: String) : Exception(message)