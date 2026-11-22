package com.waghitech.krapy

import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.ChecksumException
import com.google.zxing.DecodeHintType
import com.google.zxing.FormatException
import com.google.zxing.NotFoundException
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import com.google.zxing.PlanarYUVLuminanceSource
import java.nio.ByteBuffer

fun luminance(buf: ByteBuffer, rowStride: Int, pixelStride: Int, w: Int, h: Int): ByteArray {
    val out = ByteArray(w * h)
    val b = buf.duplicate()
    for (y in 0 until h) {
        val dst = y * w
        if (pixelStride == 1) {
            b.position(y * rowStride)
            b.get(out, dst, w)
        } else {
            var src = y * rowStride
            for (x in 0 until w) {
                out[dst + x] = b.get(src)
                src += pixelStride
            }
        }
    }
    return out
}

fun decode(lum: ByteArray, w: Int, h: Int): String? {
    val reader = QRCodeReader()
    val hints = mapOf(
        DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE),
        DecodeHintType.TRY_HARDER to true,
        DecodeHintType.CHARACTER_SET to "UTF-8",
    )
    val src = PlanarYUVLuminanceSource(lum, w, h, 0, 0, w, h, false)
    // a dark-theme terminal shows the qr inverted
    for (source in listOf(src, src.invert())) {
        try {
            reader.reset()
            return reader.decode(BinaryBitmap(HybridBinarizer(source)), hints).text
        } catch (_: NotFoundException) {
        } catch (_: ChecksumException) {
        } catch (_: FormatException) {
        }
    }
    return null
}
