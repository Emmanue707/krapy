package com.waghitech.krapy

import android.os.Looper
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetSocketAddress
import java.net.Socket
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.security.SecureRandom
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

interface LinkCallbacks {
    fun onControl(c: Wire.Control)
    fun onLinkDown(reason: String)
}

class Link(
    private var peer: Peer,
    private val phoneIdHex: String,
    private val phoneName: String,
    private val callbacks: LinkCallbacks,
) {
    private val halt = AtomicBoolean(false)
    private val transportStop = AtomicBoolean(false)
    private val connected = AtomicBoolean(false)
    private val lastPingMs = AtomicLong(0)
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
    private var pingWatchThread: Thread? = null
    private val tcpSender = Executors.newSingleThreadExecutor { r -> Thread(r, "krapy-tcp-send") }

    @Volatile
    var lastError: String? = null
        private set

    @Volatile
    var latestDesktopStats: Wire.Control.DesktopStats? = null
        private set

    private val udpPlainTail = ByteArray(Wire.MAX_VIDEO_DATA)

    fun isConnected(): Boolean = connected.get()

    // adb can only tunnel tcp, so a loopback peer means usb: video goes over the tcp link
    @Volatile
    var videoOverTcp: Boolean = false
        private set

    fun maxChunk(): Int = if (videoOverTcp) Wire.MAX_TCP_VIDEO else Wire.MAX_VIDEO_DATA

    fun peer(): Peer = peer

    fun updatePeer(p: Peer) {
        peer = p
    }

    fun connect(
        start: Wire.Control.Start,
        host: String? = null,
        port: Int? = null,
    ): Boolean {
        val h = host ?: peer.host
        val p = port ?: peer.port
        closeTransport()
        transportStop.set(false)
        lastError = null
        videoOverTcp = isLoopback(h)
        try {
            val socket = Socket()
            socket.sendBufferSize = 1024 * 1024
            socket.trafficClass = 0xB8
            socket.connect(InetSocketAddress(h, p), 2000)
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
                lastError = "desktop closed the connection before answering"
                return false
            }
            val wlen = Wire.readTcpFrameLength(tcpLenBuf)
            if (wlen < 0 || wlen > 65535) {
                socket.close()
                lastError = "bad reply from desktop"
                return false
            }
            val welcomeBody = ByteArray(wlen)
            var got = 0
            while (got < wlen) {
                val n = inp.read(welcomeBody, got, wlen - got)
                if (n < 0) {
                    socket.close()
                    lastError = "desktop closed the connection mid-handshake"
                    return false
                }
                got += n
            }
            val (desktopId, nonceD) = Wire.decodeWelcome(welcomeBody)
            if (Wire.hexId(desktopId) != peer.id) {
                socket.close()
                lastError = "a different desktop answered at $h:$p, pair again"
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
            val seal = tcpSeal!!
            synchronized(tcpSendLock) {
                val frame = Wire.encodeTcpFrame(seal.sealTcp(startBytes))
                out.write(frame)
                out.flush()
            }
            val dg = DatagramSocket()
            dg.sendBufferSize = 1024 * 1024
            dg.trafficClass = 0xB8
            dg.connect(InetSocketAddress(h, p))
            udp = dg
            writeUdpKeepalive(dg, udpSeal!!)
            connected.set(true)
            lastPingMs.set(System.currentTimeMillis())
            startReaderThreads()
            if (!videoOverTcp) {
                startKeepalive()
                startPingWatch()
            }
            peer = peer.copy(host = h, port = p)
            return true
        } catch (e: Exception) {
            closeTransport()
            lastError = describe(e, h, p)
            return false
        }
    }

    private fun isLoopback(h: String): Boolean =
        h == "127.0.0.1" || h == "localhost" || h == "::1"

    private fun describe(e: Exception, h: String, p: Int): String = when (e) {
        is java.net.ConnectException -> "nothing listening at $h:$p (is krapy running on the PC? firewall?)"
        is java.net.SocketTimeoutException -> "no answer from $h:$p (same Wi-Fi? firewall?)"
        is java.net.NoRouteToHostException -> "no route to $h (different network?)"
        else -> "${e.javaClass.simpleName}: ${e.message ?: "connect failed"}"
    }

    fun sendFormat(width: Int, height: Int) {
        sendTcpSealed(Wire.encodeControl(Wire.Control.Format(width, height)))
    }

    fun sendPreset(id: Int) = sendTcpSealed(Wire.encodePreset(id))

    fun sendPaused(on: Boolean) = sendTcpSealed(Wire.encodePaused(on))

    fun sendRotate() = sendTcpSealed(Wire.encodeRotate())

    fun sendAudioState(on: Boolean) = sendTcpSealed(Wire.encodeAudioState(on))

    fun sendPhoneStats(encAvgUs: Int, encMaxUs: Int, fps: Int, bitrate: Int) {
        sendTcpSealed(Wire.encodePhoneStats(encAvgUs, encMaxUs, fps, bitrate))
    }

    fun sendAudio(seq: Int, ptsUs: Long, buf: ByteArray, off: Int, len: Int) {
        if (!connected.get()) return
        if (videoOverTcp) {
            sendTcpSealedDirect(Wire.encodeAudioChunk(seq, ptsUs, buf, off, len))
            return
        }
        val seal = udpSeal ?: return
        val dg = udp ?: return
        val body = Wire.encodeAudioDatagram(seq, ptsUs, buf, off, len)
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
        if (!connected.get()) return
        if (videoOverTcp) {
            sendTcpSealedDirect(Wire.encodeVideoChunk(frameId, idx, count, flags, captureUs, data, off, len))
            return
        }
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
            val plain = if (plainLen <= udpPlainTail.size) {
                System.arraycopy(body, 1, udpPlainTail, 0, plainLen)
                udpPlainTail.copyOf(plainLen)
            } else {
                body.copyOfRange(1, bodyLen)
            }
            putU64(header, 1, seal.counter())
            val wire = seal.sealUdp(header.copyOfRange(0, 9), plain)
            try {
                dg.send(DatagramPacket(wire, wire.size))
            } catch (_: Exception) {
            }
        }
    }

    fun sendTcpSealed(plaintext: ByteArray) {
        if (!connected.get()) return
        if (Looper.myLooper() == Looper.getMainLooper()) {
            tcpSender.execute { Workers.guard("krapy-tcp-send") { sendTcpSealedDirect(plaintext) } }
            return
        }
        sendTcpSealedDirect(plaintext)
    }

    fun close() {
        halt.set(true)
        connected.set(false)
        closeTransport()
        tcpSender.shutdown()
    }

    fun closeTransport() {
        transportStop.set(true)
        connected.set(false)
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
        tcpSeal = null
        tcpOpen = null
        udpSeal = null
        udpOpen = null
        tcpThread = null
        udpThread = null
        keepaliveThread = null
        pingWatchThread = null
    }

    private fun sendTcpSealedDirect(plaintext: ByteArray) {
        if (!connected.get()) return
        val seal = tcpSeal ?: return
        val out = tcpOut ?: return
        try {
            synchronized(tcpSendLock) {
                val frame = Wire.encodeTcpFrame(seal.sealTcp(plaintext))
                out.write(frame)
                out.flush()
            }
        } catch (_: Exception) {
            if (!halt.get()) onTransportLost("tcp write")
        }
    }

    private fun writeUdpKeepalive(dg: DatagramSocket, seal: Sealer) {
        val body = Wire.encodeDatagramBody(Wire.DG_KEEPALIVE)
        val header = udpWireBuf
        synchronized(udpSendLock) {
            header[0] = body[0]
            putU64(header, 1, seal.counter())
            val wire = seal.sealUdp(header.copyOfRange(0, 9), ByteArray(0))
            try {
                dg.send(DatagramPacket(wire, wire.size))
            } catch (_: Exception) {
            }
        }
    }

    private fun onTransportLost(reason: String) {
        if (halt.get()) return
        if (!connected.getAndSet(false)) return
        lastError = reason
        closeTransport()
        callbacks.onLinkDown(reason)
    }

    private fun startReaderThreads() {
        tcpThread = Workers.start("krapy-tcp") { tcpLoop() }
        udpThread = Workers.start("krapy-udp") { udpLoop() }
    }

    private fun startKeepalive() {
        keepaliveThread = Workers.start("krapy-ka") {
            while (!transportStop.get() && !halt.get()) {
                try {
                    Thread.sleep(1000)
                } catch (_: InterruptedException) {
                    break
                }
                if (transportStop.get() || halt.get()) break
                if (!connected.get()) continue
                sendUdpRaw(Wire.DG_KEEPALIVE)
            }
        }
    }

    private fun startPingWatch() {
        pingWatchThread = Workers.start("krapy-ping") {
            while (!transportStop.get() && !halt.get()) {
                try {
                    Thread.sleep(500)
                } catch (_: InterruptedException) {
                    break
                }
                if (transportStop.get() || halt.get()) break
                if (!connected.get()) continue
                val last = lastPingMs.get()
                if (last > 0 && System.currentTimeMillis() - last > 3000) {
                    onTransportLost("ping timeout")
                    break
                }
            }
        }
    }

    private fun tcpLoop() {
        val inp = tcpIn ?: return
        val open = tcpOpen ?: return
        val lenBuf = ByteArray(2)
        while (!transportStop.get() && !halt.get()) {
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
                if (ctrl is Wire.Control.DesktopStats) {
                    latestDesktopStats = ctrl
                }
                callbacks.onControl(ctrl)
            } catch (_: Exception) {
                break
            }
        }
        if (!transportStop.get() && !halt.get()) {
            onTransportLost("tcp closed")
        }
    }

    private fun udpLoop() {
        val dg = udp ?: return
        val open = udpOpen ?: return
        val packet = DatagramPacket(ByteArray(1400), 1400)
        while (!transportStop.get() && !halt.get()) {
            try {
                dg.receive(packet)
                val wire = packet.data.copyOfRange(0, packet.length)
                val plain = try {
                    open.openUdp(wire)
                } catch (_: CryptoException) {
                    continue
                }
                if (wire[0] == Wire.DG_PING.toByte() && plain.size >= 12) {
                    lastPingMs.set(System.currentTimeMillis())
                    val id = u32(plain, 0)
                    val t = u64(plain, 4)
                    sendUdpPong(id, t)
                }
            } catch (_: Exception) {
                if (transportStop.get() || halt.get()) break
            }
        }
    }

    private fun sendUdpPong(id: Int, tUs: Long) {
        val phoneUs = System.nanoTime() / 1000
        sendUdpRaw(Wire.DG_PONG, pingId = id, pingT = tUs, phoneUs = phoneUs)
    }

    private fun sendUdpRaw(type: Int, pingId: Int = 0, pingT: Long = 0, phoneUs: Long = 0) {
        if (!connected.get()) return
        val seal = udpSeal ?: return
        val dg = udp ?: return
        val body = Wire.encodeDatagramBody(type, pingId = pingId, pingT = pingT, phoneUs = phoneUs)
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