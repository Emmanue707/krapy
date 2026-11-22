package com.waghitech.krapy

import org.junit.Assert.assertArrayEquals
import org.junit.Test

class HkdfTest {
    @Test
    fun rfc5869Case1() {
        val ikm = ByteArray(22) { 0x0b }
        val salt = byteArrayOf(
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x08, 0x09, 0x0a, 0x0b, 0x0c,
        )
        val info = byteArrayOf(
            0xf0.toByte(), 0xf1.toByte(), 0xf2.toByte(), 0xf3.toByte(), 0xf4.toByte(),
            0xf5.toByte(), 0xf6.toByte(), 0xf7.toByte(), 0xf8.toByte(), 0xf9.toByte(),
        )
        val expected = byteArrayOf(
            0x3c.toByte(), 0xb2.toByte(), 0x5f.toByte(), 0x25.toByte(), 0xfa.toByte(), 0xac.toByte(),
            0xd5.toByte(), 0x7a.toByte(), 0x90.toByte(), 0x43.toByte(), 0x4f.toByte(), 0x64.toByte(),
            0xd0.toByte(), 0x36.toByte(), 0x2f.toByte(), 0x2a.toByte(), 0x2d.toByte(), 0x2d.toByte(),
            0x0a.toByte(), 0x90.toByte(), 0xcf.toByte(), 0x1a.toByte(), 0x5a.toByte(), 0x4c.toByte(),
            0x5d.toByte(), 0xb0.toByte(), 0x2d.toByte(), 0x56.toByte(), 0xec.toByte(), 0xc4.toByte(),
            0xc5.toByte(), 0xbf.toByte(), 0x34.toByte(), 0x00.toByte(), 0x72.toByte(), 0x08.toByte(),
            0xd5.toByte(), 0xb8.toByte(), 0x87.toByte(), 0x18.toByte(), 0x58.toByte(), 0x65.toByte(),
        )
        val okm = Hkdf.sha256(ikm, salt, info, 42)
        assertArrayEquals(expected, okm)
    }
}