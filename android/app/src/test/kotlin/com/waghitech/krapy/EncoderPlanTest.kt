package com.waghitech.krapy

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class EncoderPlanTest {
    @Test
    fun capLongSide() {
        val (w, h) = EncoderPlan.pickLongSideCap(1440, 3200, 2400)
        assertEquals(2400, maxOf(w, h))
    }

    @Test
    fun nativeUnderCap() {
        val (w, h) = EncoderPlan.pickLongSideCap(1080, 2400, 2400)
        assertEquals(1080 to 2400, w to h)
    }

    @Test
    fun levelPortrait60() {
        assertEquals(0x8000, EncoderPlan.pickLevel(1080, 2400, 60))
        assertEquals(0x400, EncoderPlan.pickLevel(1280, 720, 60))
        assertEquals(0x2000, EncoderPlan.pickLevel(1920, 1080, 60))
    }

    @Test
    fun pickSizeAligns() {
        val (w, h) = EncoderPlan.pickSize(
            1080,
            2400,
            2400,
            64,
            64,
        ) { _, _ -> true }
        assertEquals(0, w % 64)
        assertEquals(0, h % 64)
    }
}