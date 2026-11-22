package com.waghitech.krapy

import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertEquals
import org.junit.Test
import java.nio.ByteBuffer

class QrDecodeTest {
    private val sampleUri =
        "krapy://pair?v=1&h=10.98.0.12&p=27183&k=0sillF48Q1RI86UI2AioS_QkkfCY95LDlGWmaDYCRDo&id=f72cd30ccd34ad93&n=wcp"

    private fun raster(uri: String, invert: Boolean): Pair<ByteArray, Int> {
        val hints = mapOf(EncodeHintType.MARGIN to 4, EncodeHintType.CHARACTER_SET to "UTF-8")
        val matrix = QRCodeWriter().encode(uri, BarcodeFormat.QR_CODE, 0, 0, hints)
        val modules = matrix.width
        val scale = 4
        val size = modules * scale
        val lum = ByteArray(size * size)
        for (y in 0 until size) {
            for (x in 0 until size) {
                val dark = matrix.get(x / scale, y / scale)
                val v = if (invert) {
                    if (dark) 255 else 0
                } else {
                    if (dark) 0 else 255
                }
                lum[y * size + x] = v.toByte()
            }
        }
        return lum to size
    }

    private fun rotate90Cw(lum: ByteArray, w: Int, h: Int): Pair<ByteArray, Int> {
        val out = ByteArray(w * h)
        for (y in 0 until h) {
            for (x in 0 until w) {
                out[x * h + (h - 1 - y)] = lum[y * w + x]
            }
        }
        return out to h
    }

    private fun packRowStride(lum: ByteArray, w: Int, h: Int, rowStride: Int): ByteBuffer {
        val rows = h - 1
        val size = rowStride * rows + w
        val buf = ByteBuffer.allocate(size)
        for (y in 0 until h) {
            val off = y * w
            for (x in 0 until w) {
                buf.put(lum[off + x])
            }
            if (y < h - 1) {
                var pad = rowStride - w
                while (pad-- > 0) buf.put(0)
            }
        }
        buf.position(0)
        return buf
    }

    @Test
    fun decodeNormal() {
        val (lum, size) = raster(sampleUri, invert = false)
        val text = decode(lum, size, size)
        assertEquals(sampleUri, text)
        Pairing.parse(text!!)
    }

    @Test
    fun decodeInverted() {
        val (lum, size) = raster(sampleUri, invert = true)
        val text = decode(lum, size, size)
        assertEquals(sampleUri, text)
        Pairing.parse(text!!)
    }

    @Test
    fun luminanceShortLastRow() {
        val (lum, size) = raster(sampleUri, invert = false)
        val rowStride = size + 64
        val buf = packRowStride(lum, size, size, rowStride)
        val tight = luminance(buf, rowStride, 1, size, size)
        assertEquals(lum.size, tight.size)
        val text = decode(tight, size, size)
        assertEquals(sampleUri, text)
    }

    @Test
    fun decodeRotated90() {
        val (lum, size) = raster(sampleUri, invert = false)
        val (rot, side) = rotate90Cw(lum, size, size)
        val text = decode(rot, side, side)
        assertNotNull(text)
        assertEquals(sampleUri, text)
        Pairing.parse(text!!)
    }
}