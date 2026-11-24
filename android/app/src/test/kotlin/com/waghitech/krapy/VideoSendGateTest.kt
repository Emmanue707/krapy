package com.waghitech.krapy

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class VideoSendGateTest {
    @Test
    fun dropUntilKey() {
        assertFalse(VideoSendGate.shouldSend(0, true))
        assertTrue(VideoSendGate.shouldSend(Wire.VIDEO_FLAG_KEYFRAME, true))
    }

    @Test
    fun clearsOnKeyframe() {
        assertFalse(VideoSendGate.onSent(Wire.VIDEO_FLAG_KEYFRAME, true))
    }

    @Test
    fun oldPFramesAreDroppedButNeverKeyframes() {
        assertTrue(VideoSendGate.tooOld(0, 80_000))
        assertFalse(VideoSendGate.tooOld(0, 20_000))
        assertFalse(VideoSendGate.tooOld(VideoSendGate.KEYFRAME, 500_000))
    }
}
