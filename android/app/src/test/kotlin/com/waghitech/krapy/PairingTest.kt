package com.waghitech.krapy

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test
import java.nio.charset.StandardCharsets
import java.util.Base64

class PairingTest {
    private fun b64Url(bytes: ByteArray): String {
        return Base64.getUrlEncoder().withoutPadding().encodeToString(bytes)
    }

    @Test
    fun parseValid() {
        val k = ByteArray(32) { it.toByte() }
        val id = "0123456789abcdef"
        val uri = "krapy://pair?v=1&h=192.168.0.4&p=27183&k=${b64Url(k)}&id=$id&n=desk"
        val data = Pairing.parse(uri)
        assertEquals("192.168.0.4", data.host)
        assertEquals(27183, data.port)
        assertEquals(id, data.desktopId)
        assertEquals("desk", data.desktopName)
        assertEquals(32, data.secret.size)
        val salt = "krapy-v1".toByteArray(StandardCharsets.UTF_8)
        val info = "pair".toByteArray(StandardCharsets.UTF_8)
        assertArrayEquals(Hkdf.sha256(k, salt, info, 32), data.secret)
    }

    @Test(expected = PairingException::class)
    fun rejectBadVersion() {
        val k = b64Url(ByteArray(32))
        Pairing.parse("krapy://pair?v=2&h=h&p=1&k=$k&id=0123456789abcdef&n=x")
    }

    @Test(expected = PairingException::class)
    fun rejectShortKey() {
        val k = b64Url(ByteArray(16))
        Pairing.parse("krapy://pair?v=1&h=h&p=1&k=$k&id=0123456789abcdef&n=x")
    }

    @Test(expected = PairingException::class)
    fun rejectBadId() {
        val k = b64Url(ByteArray(32))
        Pairing.parse("krapy://pair?v=1&h=h&p=1&k=$k&id=short&n=x")
    }
}