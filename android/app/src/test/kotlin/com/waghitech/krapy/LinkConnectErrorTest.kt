package com.waghitech.krapy

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.ServerSocket

class LinkConnectErrorTest {
    private val callbacks = object : LinkCallbacks {
        override fun onControl(c: Wire.Control) {}
        override fun onLinkDown(reason: String) {}
    }
    private val peer = Peer("abababababababab", "desk", ByteArray(32), "127.0.0.1", 0, 0)
    private val start = Wire.Control.Start(640, 480, 30, 2_000_000, 1)

    @Test
    fun nothingListening() {
        val port = ServerSocket(0).use { it.localPort }
        val link = Link(peer, "0123456789abcdef", "phone", callbacks)
        assertFalse(link.connect(start, "127.0.0.1", port))
        assertTrue(link.lastError!!, link.lastError!!.startsWith("nothing listening"))
        link.close()
    }

    @Test
    fun desktopHangsUp() {
        val server = ServerSocket(0)
        val t = Workers.start("fake-desk") { server.accept().close() }
        val link = Link(peer, "0123456789abcdef", "phone", callbacks)
        assertFalse(link.connect(start, "127.0.0.1", server.localPort))
        assertTrue(link.lastError!!, link.lastError!!.contains("closed the connection"))
        link.close()
        t.join(2000)
        server.close()
    }
}
