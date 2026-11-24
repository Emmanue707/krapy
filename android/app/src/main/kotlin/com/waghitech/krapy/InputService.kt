package com.waghitech.krapy

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.graphics.Path
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.accessibility.AccessibilityNodeInfo

class InputService : AccessibilityService() {
    private var videoW = 1
    private var videoH = 1
    private var displayW = 1
    private var displayH = 1
    private val planner = TouchGesturePlanner()
    private val handler = Handler(Looper.getMainLooper())
    private var activeStroke: GestureDescription.StrokeDescription? = null

    override fun onServiceConnected() {
        super.onServiceConnected()
        InputBridge.service = this
    }

    override fun onDestroy() {
        if (InputBridge.service === this) InputBridge.service = null
        super.onDestroy()
    }

    override fun onAccessibilityEvent(event: android.view.accessibility.AccessibilityEvent?) {}

    override fun onInterrupt() {}

    fun setVideoSize(vw: Int, vh: Int, dw: Int, dh: Int) {
        videoW = vw.coerceAtLeast(1)
        videoH = vh.coerceAtLeast(1)
        displayW = dw.coerceAtLeast(1)
        displayH = dh.coerceAtLeast(1)
    }

    fun onTouch(action: Int, x: Int, y: Int) {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            onTouchMain(action, x, y)
        } else {
            handler.post { onTouchMain(action, x, y) }
        }
    }

    private fun onTouchMain(action: Int, x: Int, y: Int) {
        val mx = x * displayW.toFloat() / videoW
        val my = y * displayH.toFloat() / videoH
        val now = System.currentTimeMillis()
        val seg =
            when (action) {
                0 -> planner.onDown(mx, my, now)
                1 -> planner.onMove(mx, my, now)
                2 -> planner.onUp(mx, my, now)
                else -> null
            }
        if (seg != null) {
            dispatchSegment(seg, now)
        }
    }

    fun onKey(code: Int) {
        when (code) {
            1 -> performGlobalAction(GLOBAL_ACTION_BACK)
            2 -> performGlobalAction(GLOBAL_ACTION_HOME)
            3 -> performGlobalAction(GLOBAL_ACTION_RECENTS)
            4 -> {
                if (Build.VERSION.SDK_INT >= 28) {
                    performGlobalAction(GLOBAL_ACTION_LOCK_SCREEN)
                }
            }
        }
    }

    fun onText(text: String) {
        val root = rootInActiveWindow ?: return
        val focus = root.findFocus(AccessibilityNodeInfo.FOCUS_INPUT) ?: return
        val args = Bundle()
        args.putCharSequence(
            AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
            text,
        )
        focus.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, args)
    }

    private fun dispatchSegment(seg: GestureSegment, nowMs: Long) {
        val path =
            Path().apply {
                moveTo(seg.fromX, seg.fromY)
                lineTo(seg.toX, seg.toY)
            }
        val stroke =
            if (seg.freshDown || activeStroke == null) {
                GestureDescription.StrokeDescription(path, 0, seg.durationMs, seg.willContinue)
            } else {
                activeStroke!!.continueStroke(path, 0, seg.durationMs, seg.willContinue)
            }
        if (!seg.willContinue) {
            activeStroke = null
        } else {
            activeStroke = stroke
        }
        val g = GestureDescription.Builder().addStroke(stroke).build()
        planner.onSegmentDispatched(nowMs)
        val cb =
            object : GestureResultCallback() {
                override fun onCompleted(gestureDescription: GestureDescription?) {
                    val next = planner.onSegmentComplete(System.currentTimeMillis())
                    if (next != null) {
                        dispatchSegment(next, System.currentTimeMillis())
                    }
                }

                override fun onCancelled(gestureDescription: GestureDescription?) {
                    activeStroke = null
                    val next = planner.onSegmentCancelled(System.currentTimeMillis())
                    if (next != null) {
                        dispatchSegment(next, System.currentTimeMillis())
                    }
                }
            }
        dispatchGesture(g, cb, handler)
    }

    object InputBridge {
        @Volatile
        var service: InputService? = null

        fun enabled(): Boolean = service != null

        fun setVideoSize(vw: Int, vh: Int, dw: Int, dh: Int) {
            service?.setVideoSize(vw, vh, dw, dh)
        }

        fun touch(action: Int, pointer: Int, x: Int, y: Int) {
            if (pointer != 0) return
            service?.onTouch(action, x, y)
        }

        fun key(code: Int) {
            service?.onKey(code)
        }

        fun text(s: String) {
            service?.onText(s)
        }
    }
}