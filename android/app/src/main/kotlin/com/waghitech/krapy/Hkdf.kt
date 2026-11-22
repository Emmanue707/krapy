package com.waghitech.krapy

import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

object Hkdf {
    fun sha256(
        ikm: ByteArray,
        salt: ByteArray,
        info: ByteArray,
        length: Int,
    ): ByteArray {
        require(length > 0) { "length must be positive" }
        val prk = extract(ikm, salt)
        return expand(prk, info, length)
    }

    private fun extract(ikm: ByteArray, salt: ByteArray): ByteArray {
        val mac = Mac.getInstance("HmacSHA256")
        mac.init(SecretKeySpec(salt, "HmacSHA256"))
        return mac.doFinal(ikm)
    }

    private fun expand(prk: ByteArray, info: ByteArray, length: Int): ByteArray {
        val hashLen = 32
        val n = (length + hashLen - 1) / hashLen
        val out = ByteArray(length)
        var t = ByteArray(0)
        var offset = 0
        for (i in 1..n) {
            val mac = Mac.getInstance("HmacSHA256")
            mac.init(SecretKeySpec(prk, "HmacSHA256"))
            if (t.isNotEmpty()) mac.update(t)
            mac.update(info)
            mac.update(byteArrayOf(i.toByte()))
            t = mac.doFinal()
            val copy = minOf(hashLen, length - offset)
            System.arraycopy(t, 0, out, offset, copy)
            offset += copy
        }
        return out
    }
}