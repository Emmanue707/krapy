package com.waghitech.krapy

import org.junit.Assert.assertEquals
import org.junit.Test

class SessionPrefsTest {
    @Test
    fun presetLabels() {
        assertEquals("Smooth", SessionPrefs.presetLabel(SessionPrefs.PRESET_SMOOTH))
        assertEquals("Balanced", SessionPrefs.presetLabel(SessionPrefs.PRESET_BALANCED))
        assertEquals("Sharp", SessionPrefs.presetLabel(SessionPrefs.PRESET_SHARP))
    }

    @Test
    fun presetCoerce() {
        assertEquals("Sharp", SessionPrefs.presetLabel(99))
        assertEquals("Smooth", SessionPrefs.presetLabel(-1))
    }

    @Test
    fun presetBitrates() {
        assertEquals(8_000_000, SessionPrefs.presetQuality(SessionPrefs.PRESET_SMOOTH).bitrate)
        assertEquals(16_000_000, SessionPrefs.presetQuality(SessionPrefs.PRESET_BALANCED).bitrate)
        assertEquals(24_000_000, SessionPrefs.presetQuality(SessionPrefs.PRESET_SHARP).bitrate)
    }
}