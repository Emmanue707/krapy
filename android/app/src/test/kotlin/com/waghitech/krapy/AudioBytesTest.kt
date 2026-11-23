package com.waghitech.krapy

import org.junit.Assert.assertEquals
import org.junit.Test

class AudioBytesTest {
    @Test
    fun chunkSize() {
        assertEquals(960, Wire.MAX_AUDIO_DATA)
        assertEquals(960, 240 * 2 * 2)
    }
}