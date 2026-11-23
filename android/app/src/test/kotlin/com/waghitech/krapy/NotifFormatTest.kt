package com.waghitech.krapy

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class NotifFormatTest {
    @Test
    fun statsLine() {
        val s = SessionState.Snap(fps = 60, latencyMs = 18, bitrateBps = 8_100_000)
        val t = NotifFormat.contentText(s)
        assertTrue(t.contains("60 fps"))
        assertTrue(t.contains("18 ms"))
        assertTrue(t.contains("8.1 Mbps"))
    }

    @Test
    fun actionLabels() {
        assertEquals("Sound: PC", NotifFormat.soundActionLabel(true))
        assertEquals("Sound: phone", NotifFormat.soundActionLabel(false))
        assertEquals("Pause", NotifFormat.pauseActionLabel(false))
        assertEquals("Resume", NotifFormat.pauseActionLabel(true))
    }
}