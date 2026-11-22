package com.waghitech.krapy

import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetSocketAddress
import java.net.Socket
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.security.SecureRandom
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread

interface LinkCallbacks {
    fun onControl(c: Wire.Control)
    fun onLost(reason: String)
}

class Link(
    private val peer: Peer,
    private val phoneIdHex: String,
    private val phoneName: String,
    private val callbacks: LinkCallbacks,
) {
    private val stop = AtomicBoolean(false)
    private var tcpIn: BufferedInputStream? = null
    private var tcpOut: BufferedOutputStream? = null
    private var udp: DatagramSocket? = null
    private var tcpSeal: Sealer? = null
    private var tcpOpen: Opener? = null
    private var udpSeal: Sealer? = null
    private var udpOpen: Opener? = null
    private val udpSendLock = Any()
    private val tcpSendLock = Any()
    private val udpPlainBuf = ByteArray(1400)
    private val udpWireBuf = ByteArray(1400)
    private val tcpLenBuf = ByteArray(2)
    private var tcpThread: Thread? = null
    private var udpThread: Thread? = null
    private var keepaliveThread: Thread? = null

    fun connect(start: Wire.Control.Start): Boolean {
        try {
            val socket = Socket()
            socket.connect(InetSocketAddress(peer.host, peer.port), 2000)
            socket.tcpNoDelay = true
            val inp = BufferedInputStream(socket.getInputStream())
            val out = BufferedOutputStream(socket.getOutputStream())
            val nonceP = ByteArray(16)
            SecureRandom().nextBytes(nonceP)
            val hello = Wire.encodeHello(Wire.hexToBytes(phoneIdHex), nonceP, phoneName)
            out.write(Wire.encodeTcpFrame(hello))
            out.flush()
            if (inp.read(tcpLenBuf) != 2) {
                socket.close()
                callbacks.onLost("welcome length")
                return false
            }
            val wlen = Wire.readTcpFrameLength(tcpLenBuf)
            if (wlen < 0 || wlen > 65535) {
                socket.close()
                callbacks.onLost("welcome size")
                return false
            }
            val welcomeBody = ByteArray(wlen)
            var got = 0
            while (got < wlen) {
                val n = inp.read(welcomeBody, got, wlen - got)
                if (n < 0) {
                    socket.close()
                    callbacks.onLost("welcome body")
                    return false
                }
                got += n
            }
            val (desktopId, nonceD) = Wire.decodeWelcome(welcomeBody)
            if (Wire.hexId(desktopId) != peer.id) {
                socket.close()
                callbacks.onLost("desktop id mismatch")
                return false
            }
            val keys = SessionKeys.derive(peer.secret, nonceP, nonceD)
            tcpSeal = Sealer(keys.tcpP2d)
            tcpOpen = Opener(keys.tcpD2p)
            udpSeal = Sealer(keys.udpP2d)
            udpOpen = Opener(keys.udpD2p)
            tcpIn = inp
            tcpOut = out
            val startBytes = Wire.encodeControl(
                Wire.Control.Start(
                    start.width,
                    start.height,
                    start.fps,
                    start.bitrate,
                    start.codec,
                ),
            )
            sendTcpSealed(startBytes)
            val dg = DatagramSocket()
            dg.connect(InetSocketAddress(peer.host, peer.port))
            udp = dg
            sendUdpRaw(Wire.DG_KEEPALIVE)
            startReaderThreads()
            startKeepalive()
            return true
        } catch (e: Exception) {
            callbacks.onLost(e.message ?: "connect failed")
            closeQuiet()
            return false
        }
    }

    fun sendFormat(width: Int, height: Int) {
        sendTcpSealed(Wire.encodeControl(Wire.Control.Format(width, height)))
    }

    fun sendVideo(
        frameId: Int,
        idx: Int,
        count: Int,
        flags: Int,
        captureUs: Long,
        data: ByteArray,
        off: Int,
        len: Int,
    ) {
        val seal = udpSeal ?: return
        val dg = udp ?: return
        val bodyLen = 18 + len
        val body = udpPlainBuf
        body[0] = Wire.DG_VIDEO.toByte()
        putU32(body, 1, frameId)
        putU16(body, 5, idx)
        putU16(body, 7, count)
        body[9] = flags.toByte()
        putU64(body, 10, captureUs)
        System.arraycopy(data, off, body, 18, len)
        val header = udpWireBuf
        header[0] = body[0]
        synchronized(udpSendLock) {
            val plainLen = bodyLen - 1
            val plain = if (plainLen == 0) ByteArray(0) else body.copyOfRange(1, bodyLen)
            putU64(header, 1, seal.counter())
            val wire = seal.sealUdp(header.copyOfRange(0, 9), plain)
            try {
                dg.send(DatagramPacket(wire, wire.size))
            } catch (_: Exception) {
            }
        }
    }

    fun sendTcpSealed(plaintext: ByteArray) {
        val seal = tcpSeal ?: return
        val out = tcpOut ?: return
        try {
            synchronized(tcpSendLock) {
                val frame = Wire.encodeTcpFrame(seal.sealTcp(plaintext))
                synchronized(out) {
                    out.write(frame)
                    out.flush()
                }
            }
        } catch (_: Exception) {
            if (!stop.get()) callbacks.onLost("tcp write")
        }
    }

    fun close() {
        stop.set(true)
        closeQuiet()
    }

    private fun closeQuiet() {
        try {
            tcpIn?.close()
        } catch (_: Exception) {
        }
        try {
            tcpOut?.close()
        } catch (_: Exception) {
        }
        try {
            udp?.close()
        } catch (_: Exception) {
        }
        tcpIn = null
        tcpOut = null
        udp = null
    }

    private fun startReaderThreads() {
        tcpThread = thread(name = "krapy-tcp", isDaemon = true) { tcpLoop() }
        udpThread = thread(name = "krapy-udp", isDaemon = true) { udpLoop() }
    }

    private fun startKeepalive() {
        keepaliveThread = thread(name = "krapy-ka", isDaemon = true) {
            while (!stop.get()) {
                try {
                    Thread.sleep(1000)
                } catch (_: InterruptedException) {
                    break
                }
                if (stop.get()) break
                sendUdpRaw(Wire.DG_KEEPALIVE)
            }
        }
    }

    private fun tcpLoop() {
        val inp = tcpIn ?: return
        val open = tcpOpen ?: return
        val lenBuf = ByteArray(2)
        while (!stop.get()) {
            try {
                if (inp.read(lenBuf) != 2) break
                val flen = Wire.readTcpFrameLength(lenBuf)
                if (flen < 0) break
                val wire = ByteArray(flen)
                var pos = 0
                while (pos < flen) {
                    val n = inp.read(wire, pos, flen - pos)
                    if (n < 0) break
                    pos += n
                }
                if (pos < flen) break
                val pt = try {
                    open.openTcp(wire)
                } catch (_: CryptoException) {
                    continue
                }
                val ctrl = try {
                    Wire.decodeControl(pt)
                } catch (_: WireException) {
                    continue
                }
                callbacks.onControl(ctrl)
            } catch (_: Exception) {
                break
            }
        }
        if (!stop.getAndSet(true)) {
            callbacks.onLost("tcp closed")
        }
        closeQuiet()
    }

    private fun udpLoop() {
        val dg = udp ?: return
        val open = udpOpen ?: return
        val packet = DatagramPacket(ByteArray(1400), 1400)
        while (!stop.get()) {
            try {
                dg.receive(packet)
                val wire = packet.data.copyOfRange(0, packet.length)
                val plain = try {
                    open.openUdp(wire)
                } catch (_: CryptoException) {
                    continue
                }
                if (wire[0] == Wire.DG_PING.toByte() && plain.size >= 12) {
                    val id = u32(plain, 0)
                    val t = u64(plain, 4)
                    sendUdpPong(id, t)
                }
            } catch (_: Exception) {
                if (stop.get()) break
            }
        }
    }

    private fun sendUdpPong(id: Int, tUs: Long) {
        sendUdpRaw(Wire.DG_PONG, pingId = id, pingT = tUs)
    }

    private fun sendUdpRaw(type: Int, pingId: Int = 0, pingT: Long = 0) {
        val seal = udpSeal ?: return
        val dg = udp ?: return
        val body = Wire.encodeDatagramBody(type, pingId = pingId, pingT = pingT)
        val header = udpWireBuf
        synchronized(udpSendLock) {
            header[0] = body[0]
            putU64(header, 1, seal.counter())
            val plain = if (body.size <= 1) ByteArray(0) else body.copyOfRange(1, body.size)
            val wire = seal.sealUdp(header.copyOfRange(0, 9), plain)
            try {
                dg.send(DatagramPacket(wire, wire.size))
            } catch (_: Exception) {
            }
        }
    }

    private fun putU16(buf: ByteArray, off: Int, v: Int) {
        buf[off] = (v shr 8).toByte()
        buf[off + 1] = v.toByte()
    }

    private fun putU32(buf: ByteArray, off: Int, v: Int) {
        buf[off] = (v shr 24).toByte()
        buf[off + 1] = (v shr 16).toByte()
        buf[off + 2] = (v shr 8).toByte()
        buf[off + 3] = v.toByte()
    }

    private fun putU64(buf: ByteArray, off: Int, v: Long) {
        ByteBuffer.wrap(buf, off, 8).order(ByteOrder.BIG_ENDIAN).putLong(v)
    }

    private fun u32(buf: ByteArray, off: Int): Int =
        ((buf[off].toInt() and 0xff) shl 24) or
            ((buf[off + 1].toInt() and 0xff) shl 16) or
            ((buf[off + 2].toInt() and 0xff) shl 8) or
            (buf[off + 3].toInt() and 0xff)

    private fun u64(buf: ByteArray, off: Int): Long =
        ByteBuffer.wrap(buf, off, 8).order(ByteOrder.BIG_ENDIAN).long
}