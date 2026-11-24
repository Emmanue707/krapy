package com.waghitech.krapy

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AudioCaptureTest {
    @Test
    fun chunkerCarriesPartial() {
        val carry = ByteArray(Wire.MAX_AUDIO_DATA)
        val emitted = mutableListOf<ByteArray>()
        var clen = AudioChunker.feed(ByteArray(400) { it.toByte() }, 400, carry, 0, 960) {
            emitted.add(it.copyOf())
        }
        assertEquals(0, emitted.size)
        assertEquals(400, clen)
        val tail = ByteArray(600) { (it + 100).toByte() }
        clen = AudioChunker.feed(tail, 600, carry, clen, 960) { emitted.add(it.copyOf()) }
        assertEquals(1, emitted.size)
        assertEquals(40, clen)
        clen = AudioChunker.feed(ByteArray(920) { (it + 200).toByte() }, 920, carry, clen, 960) {
            emitted.add(it.copyOf())
        }
        assertEquals(2, emitted.size)
        assertEquals(0, clen)
    }

    @Test
    fun audioStateEncode() {
        val enc = Wire.encodeAudioState(true, 48_000, 2, true)
        assertEquals(8, enc.size)
        assertEquals(0x46, enc[0].toInt() and 0xff)
        assertEquals(1, enc[1].toInt() and 0xff)
        assertEquals(0x80, enc[2].toInt() and 0xff)
        assertEquals(0xBB, enc[3].toInt() and 0xff)
        assertEquals(0, enc[4].toInt() and 0xff)
        assertEquals(0, enc[5].toInt() and 0xff)
        assertEquals(2, enc[6].toInt() and 0xff)
        assertEquals(1, enc[7].toInt() and 0xff)
        val enc44 = Wire.encodeAudioState(false, 44_100, 2, false)
        assertArrayEquals(
            byteArrayOf(0x46, 0, 0x44.toByte(), 0xAC.toByte(), 0, 0, 2, 0),
            enc44,
        )
    }

    @Test
    fun muteCheckDecision() {
        assertFalse(AudioMuteCheck.captureAlsoSilenced(false, 0, 2000))
        assertFalse(AudioMuteCheck.captureAlsoSilenced(true, 100, 2000))
        assertFalse(AudioMuteCheck.captureAlsoSilenced(true, 0, 500))
        assertTrue(AudioMuteCheck.captureAlsoSilenced(true, 3, 1500))
    }
}