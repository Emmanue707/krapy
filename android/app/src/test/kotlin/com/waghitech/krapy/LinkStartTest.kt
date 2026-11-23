package com.waghitech.krapy

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.ServerSocket
import java.security.SecureRandom
import kotlin.concurrent.thread

class LinkStartTest {
    @Test
    fun connectSendsSealedStartAsFirstTcpFrame() {
        val desktopId = ByteArray(8) { 0xab.toByte() }
        val desktopIdHex = Wire.hexId(desktopId)
        val secret = ByteArray(32) { 7 }
        val peer = Peer(
            id = desktopIdHex,
            name = "desk",
            secret = secret,
            host = "127.0.0.1",
            port = 0,
            lastSeen = 0,
        )
        val server = ServerSocket(0)
        val port = server.localPort
        val peerOnPort = peer.copy(port = port)
        val latch = java.util.concurrent.CountDownLatch(1)
        var decoded: Wire.Control.Start? = null
        val serverThread = thread(name = "fake-desk", isDaemon = true) {
            server.soTimeout = 5000
            val sock = server.accept()
            sock.tcpNoDelay = true
            val inp = sock.getInputStream()
            val out = sock.getOutputStream()
            val lenHdr = ByteArray(2)
            if (inp.read(lenHdr) != 2) return@thread
            val helloLen = Wire.readTcpFrameLength(lenHdr)
            val helloBody = ByteArray(helloLen)
            var pos = 0
            while (pos < helloLen) {
                val n = inp.read(helloBody, pos, helloLen - pos)
                if (n < 0) return@thread
                pos += n
            }
            val nonceP = helloBody.copyOfRange(13, 29)
            val nonceD = ByteArray(16)
            SecureRandom().nextBytes(nonceD)
            val welcome = Wire.encodeWelcome(desktopId, nonceD)
            out.write(Wire.encodeTcpFrame(welcome))
            out.flush()
            val keys = SessionKeys.derive(secret, nonceP, nonceD)
            if (inp.read(lenHdr) != 2) return@thread
            val frameLen = Wire.readTcpFrameLength(lenHdr)
            val sealed = ByteArray(frameLen)
            pos = 0
            while (pos < frameLen) {
                val n = inp.read(sealed, pos, frameLen - pos)
                if (n < 0) return@thread
                pos += n
            }
            val opener = Opener(keys.tcpP2d)
            val plain = opener.openTcp(sealed)
            val ctrl = Wire.decodeControl(plain)
            assertTrue(ctrl is Wire.Control.Start)
            decoded = ctrl as Wire.Control.Start
            latch.countDown()
            sock.close()
        }
        val callbacks = object : LinkCallbacks {
            override fun onControl(c: Wire.Control) {}
            override fun onLinkDown(reason: String) {}
        }
        val link = Link(peerOnPort, "0123456789abcdef", "phone", callbacks)
        val start = Wire.Control.Start(640, 480, 30, 2_000_000, 1)
        try {
            assertTrue(link.connect(start, "127.0.0.1", port))
            latch.await()
            assertEquals(640, decoded?.width)
            assertEquals(480, decoded?.height)
            assertEquals(30, decoded?.fps)
            assertEquals(2_000_000, decoded?.bitrate)
            assertEquals(1, decoded?.codec)
        } finally {
            link.close()
            serverThread.join(2000)
            server.close()
        }
    }
}