package com.waghitech.krapy

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test

class WireRoundtripTest {
    @Test
    fun phoneStatsRoundtrip() {
        val enc = Wire.encodePhoneStats(5000, 12000, 60, 12_000_000)
        val back = Wire.decodePhoneStats(enc)
        assertEquals(listOf(5000, 12000, 60, 12_000_000), back)
    }

    @Test
    fun pongExtendedBody() {
        val out = Wire.encodeDatagramBody(
            Wire.DG_PONG,
            pingId = 9,
            pingT = 1000L,
            phoneUs = 555L,
        )
        assertEquals(21, out.size)
        val id = (out[1].toInt() and 0xff shl 24) or (out[2].toInt() and 0xff shl 16) or
            (out[3].toInt() and 0xff shl 8) or (out[4].toInt() and 0xff)
        assertEquals(9, id)
    }

    @Test
    fun desktopStatsEncode() {
        val enc = Wire.encodeDesktopStats(60, 14, 3, 5)
        assertArrayEquals(byteArrayOf(0x41, 60, 0, 14, 0, 3, 0, 5), enc)
    }

    @Test
    fun settingsEncodeAndSetSettingDecode() {
        val s = Wire.encodeSettings(
            paused = true,
            preset = 1,
            soundPc = true,
            keepAwake = false,
            clipboard = true,
            rotation = 2,
        )
        assertArrayEquals(
            byteArrayOf(0x49, 1, 1, 1, 0, 1, 2),
            s,
        )
        val set = Wire.decodeControl(byteArrayOf(0x47, PcCommands.KEY_PAUSE.toByte(), 1))
        assertEquals(Wire.Control.SetSetting(PcCommands.KEY_PAUSE, 1), set)
    }
}