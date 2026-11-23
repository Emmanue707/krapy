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
}