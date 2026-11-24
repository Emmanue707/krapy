package com.waghitech.krapy

data class GestureSegment(
    val fromX: Float,
    val fromY: Float,
    val toX: Float,
    val toY: Float,
    val durationMs: Long,
    val willContinue: Boolean,
    val freshDown: Boolean,
)

class TouchGesturePlanner {
    private var lastX = 0f
    private var lastY = 0f
    private var targetX = 0f
    private var targetY = 0f
    private var downMs = 0L
    private var lastSegmentMs = 0L
    private var active = false
    private var inFlight = false
    private var queuedMove = false
    private var pendingUp = false
    private var needFreshDown = false

    fun reset() {
        active = false
        inFlight = false
        queuedMove = false
        pendingUp = false
        needFreshDown = false
    }

    fun onDown(x: Float, y: Float, nowMs: Long): GestureSegment? {
        reset()
        active = true
        queuedMove = false
        pendingUp = false
        needFreshDown = false
        lastX = x
        lastY = y
        targetX = x
        targetY = y
        downMs = nowMs
        lastSegmentMs = nowMs
        return segment(x, y, x, y, MIN_SEGMENT_MS, true, true)
    }

    fun onMove(x: Float, y: Float, nowMs: Long): GestureSegment? {
        if (!active) return null
        targetX = x
        targetY = y
        queuedMove = true
        return if (inFlight) null else drain(nowMs)
    }

    fun onUp(x: Float, y: Float, nowMs: Long): GestureSegment? {
        if (!active) return null
        targetX = x
        targetY = y
        pendingUp = true
        queuedMove = true
        return if (inFlight) null else drain(nowMs)
    }

    fun onSegmentDispatched(nowMs: Long) {
        inFlight = true
        lastSegmentMs = nowMs
    }

    fun onSegmentComplete(nowMs: Long): GestureSegment? {
        if (!active) return null
        inFlight = false
        return drain(nowMs)
    }

    fun onSegmentCancelled(nowMs: Long): GestureSegment? {
        if (!active) return null
        inFlight = false
        needFreshDown = true
        queuedMove = true
        return drain(nowMs)
    }

    private fun drain(nowMs: Long): GestureSegment? {
        if (!active || inFlight) return null
        if (!queuedMove && !pendingUp) return null
        if (pendingUp && (!queuedMove || (targetX == lastX && targetY == lastY))) {
            return finishUp(nowMs)
        }
        queuedMove = false
        val dur = segmentDuration(lastX, lastY, targetX, targetY, lastSegmentMs, nowMs)
        val seg =
            segment(
                lastX,
                lastY,
                targetX,
                targetY,
                dur,
                true,
                needFreshDown,
            )
        needFreshDown = false
        lastX = targetX
        lastY = targetY
        inFlight = true
        lastSegmentMs = nowMs
        return seg
    }

    private fun finishUp(nowMs: Long): GestureSegment? {
        val dur = segmentDuration(lastX, lastY, targetX, targetY, lastSegmentMs, nowMs)
        val seg =
            segment(
                lastX,
                lastY,
                targetX,
                targetY,
                dur,
                false,
                needFreshDown,
            )
        active = false
        inFlight = true
        pendingUp = false
        queuedMove = false
        needFreshDown = false
        lastSegmentMs = nowMs
        return seg
    }

    companion object {
        const val MIN_SEGMENT_MS = 8L
        const val MAX_SEGMENT_MS = 50L
        const val LONG_PRESS_MS = 500L

        fun segmentDuration(
            fromX: Float,
            fromY: Float,
            toX: Float,
            toY: Float,
            lastSegmentMs: Long,
            nowMs: Long,
        ): Long {
            val elapsed = (nowMs - lastSegmentMs).coerceAtLeast(0)
            val moved = fromX != toX || fromY != toY
            if (!moved) {
                return elapsed.coerceIn(MIN_SEGMENT_MS, LONG_PRESS_MS)
            }
            return elapsed.coerceIn(MIN_SEGMENT_MS, MAX_SEGMENT_MS)
        }

        fun segment(
            fromX: Float,
            fromY: Float,
            toX: Float,
            toY: Float,
            durationMs: Long,
            willContinue: Boolean,
            freshDown: Boolean,
        ): GestureSegment =
            GestureSegment(fromX, fromY, toX, toY, durationMs, willContinue, freshDown)
    }
}