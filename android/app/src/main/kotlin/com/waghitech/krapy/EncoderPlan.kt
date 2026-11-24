package com.waghitech.krapy

import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

object EncoderPlan {
    fun pickLongSideCap(displayW: Int, displayH: Int, maxLong: Int): Pair<Int, Int> {
        val dw = max(1, displayW)
        val dh = max(1, displayH)
        val long = max(dw, dh)
        if (maxLong <= 0 || long <= maxLong) return dw to dh
        val scale = maxLong.toDouble() / long
        val w = (dw * scale).roundToInt().coerceAtLeast(2)
        val h = (dh * scale).roundToInt().coerceAtLeast(2)
        return w to h
    }

    fun alignDown(v: Int, align: Int): Int {
        if (align <= 1) return v
        return (v / align) * align
    }

    fun pickSize(
        displayW: Int,
        displayH: Int,
        maxLong: Int,
        widthAlign: Int,
        heightAlign: Int,
        sizeOk: (Int, Int) -> Boolean,
    ): Pair<Int, Int> {
        val (tw, th) = pickLongSideCap(displayW, displayH, maxLong)
        var w = alignDown(tw, widthAlign).coerceAtLeast(widthAlign)
        var h = alignDown(th, heightAlign).coerceAtLeast(heightAlign)
        if (sizeOk(w, h)) return w to h
        val candidates = listOf(
            alignDown(min(displayW, tw), widthAlign) to alignDown(min(displayH, th), heightAlign),
            alignDown(displayW, widthAlign) to alignDown(displayH, heightAlign),
        )
        for ((cw, ch) in candidates) {
            val aw = max(widthAlign, cw)
            val ah = max(heightAlign, ch)
            if (sizeOk(aw, ah)) return aw to ah
        }
        var scale = 0.9
        repeat(8) {
            val cw = alignDown((tw * scale).roundToInt(), widthAlign).coerceAtLeast(widthAlign)
            val ch = alignDown((th * scale).roundToInt(), heightAlign).coerceAtLeast(heightAlign)
            if (sizeOk(cw, ch)) return cw to ch
            scale *= 0.9
        }
        return w to h
    }

    // (max frame size in macroblocks, max macroblocks per second, MediaCodec level constant)
    private val LEVELS = arrayOf(
        Triple(3_600, 108_000, 0x200),
        Triple(5_120, 216_000, 0x400),
        Triple(8_192, 245_760, 0x800),
        Triple(8_704, 522_240, 0x2000),
        Triple(22_080, 589_824, 0x4000),
        Triple(36_864, 983_040, 0x8000),
        Triple(36_864, 2_073_600, 0x10000),
    )

    fun pickLevel(w: Int, h: Int, fps: Int): Int? {
        val frame = ((w + 15) / 16) * ((h + 15) / 16)
        val perSec = frame.toLong() * fps.coerceAtLeast(1)
        return LEVELS.firstOrNull { frame <= it.first && perSec <= it.second }?.third
    }
}