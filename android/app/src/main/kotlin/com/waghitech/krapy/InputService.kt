package com.waghitech.krapy

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.graphics.Path
import android.os.Build
import android.os.Bundle
import android.view.accessibility.AccessibilityNodeInfo

class InputService : AccessibilityService() {
    private var videoW = 1
    private var videoH = 1
    private var displayW = 1
    private var displayH = 1
    private var strokePath = Path()
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
        val mx = x * displayW.toFloat() / videoW
        val my = y * displayH.toFloat() / videoH
        when (action) {
            0 -> {
                strokePath = Path().apply { moveTo(mx, my) }
                activeStroke = GestureDescription.StrokeDescription(strokePath, 0, 1, true)
                fireGesture(activeStroke!!)
            }
            1 -> {
                val prev = activeStroke ?: return
                strokePath.lineTo(mx, my)
                val next = prev.continueStroke(strokePath, 0, 1, true)
                activeStroke = next
                fireGesture(next)
            }
            2 -> {
                val prev = activeStroke ?: return
                strokePath.lineTo(mx, my)
                val next = prev.continueStroke(strokePath, 0, 1, false)
                activeStroke = null
                fireGesture(next)
            }
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

    private fun fireGesture(stroke: GestureDescription.StrokeDescription) {
        val g = GestureDescription.Builder().addStroke(stroke).build()
        dispatchGesture(g, null, null)
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