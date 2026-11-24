package com.waghitech.krapy

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TouchGesturePlannerTest {
    @Test
    fun downStartsAtPoint() {
        val p = TouchGesturePlanner()
        val s = p.onDown(10f, 20f, 1000)
        assertNotNull(s)
        assertEquals(10f, s!!.fromX, 0f)
        assertEquals(10f, s.toX, 0f)
        assertTrue(s.willContinue)
        assertTrue(s.freshDown)
    }

    @Test
    fun segmentsChainFromLastEnd() {
        val p = TouchGesturePlanner()
        p.onDown(0f, 0f, 0)
        p.onSegmentDispatched(0)
        assertNull(p.onSegmentComplete(20))
        val drag = p.onMove(50f, 60f, 30)
        assertNotNull(drag)
        assertEquals(0f, drag!!.fromX, 0f)
        assertEquals(50f, drag.toX, 0f)
        assertEquals(60f, drag.toY, 0f)
    }

    @Test
    fun coalescesMovesWhileInFlight() {
        val p = TouchGesturePlanner()
        p.onDown(1f, 1f, 0)
        p.onSegmentDispatched(0)
        assertNull(p.onMove(2f, 2f, 5))
        assertNull(p.onMove(9f, 9f, 6))
        val seg = p.onSegmentComplete(20)
        assertNotNull(seg)
        assertEquals(9f, seg!!.toX, 0f)
        assertEquals(9f, seg.toY, 0f)
    }

    @Test
    fun upWaitsForInFlight() {
        val p = TouchGesturePlanner()
        p.onDown(0f, 0f, 0)
        p.onSegmentDispatched(0)
        p.onMove(5f, 5f, 10)
        p.onUp(5f, 5f, 11)
        val mid = p.onSegmentComplete(20)
        assertNotNull(mid)
        assertTrue(mid!!.willContinue)
        assertEquals(5f, mid.toX, 0f)
        p.onSegmentDispatched(20)
        val end = p.onSegmentComplete(30)
        assertNotNull(end)
        assertFalse(end!!.willContinue)
        assertEquals(5f, end.toX, 0f)
    }

    @Test
    fun holdWithoutMoveAllowsLongPressDuration() {
        val dur = TouchGesturePlanner.segmentDuration(1f, 1f, 1f, 1f, 0, 600)
        assertEquals(500L, dur)
    }

    @Test
    fun tapUsesShortSegment() {
        val dur = TouchGesturePlanner.segmentDuration(1f, 1f, 1f, 1f, 0, 30)
        assertEquals(30L, dur)
    }

    @Test
    fun moveClampsDuration() {
        val dur = TouchGesturePlanner.segmentDuration(0f, 0f, 10f, 0f, 0, 200)
        assertEquals(50L, dur)
    }

    @Test
    fun cancelRestartsWithFreshDown() {
        val p = TouchGesturePlanner()
        p.onDown(0f, 0f, 0)
        p.onSegmentDispatched(0)
        p.onMove(3f, 3f, 10)
        val seg = p.onSegmentCancelled(15)
        assertNotNull(seg)
        assertTrue(seg!!.freshDown)
    }
}