package com.waghitech.krapy

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.content.res.Configuration
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.DisplayMetrics
import android.view.Display
import android.view.Surface
import android.view.WindowManager
import android.widget.Toast
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.min

class MirrorService : Service(), LinkCallbacks {
    private val running = AtomicBoolean(false)
    private val reconnecting = AtomicBoolean(false)
    private val connecting = AtomicBoolean(false)
    private var projection: MediaProjection? = null
    private var virtualDisplay: VirtualDisplay? = null
    private var encoder: Encoder? = null
    private var link: Link? = null
    private var peer: Peer? = null
    private var peersDb: Peers? = null
    private var displayW: Int = 0
    private var displayH: Int = 0
    private var lastKnownDisplayW: Int = 0
    private var lastKnownDisplayH: Int = 0
    private val mainHandler = Handler(Looper.getMainLooper())
    private var projectionCallback: MediaProjection.Callback? = null
    private var displayListener: DisplayManager.DisplayListener? = null
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private var reconnectThread: Thread? = null
    private var sessionHost: String? = null
    private var sessionPort: Int? = null
    @Volatile
    private var backoffMs: Long = 250
    private var wifiLock: WifiManager.WifiLock? = null
    private var statsThread: Thread? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopSession("stopped")
            return START_NOT_STICKY
        }
        try {
            val peerId = intent?.getStringExtra(EXTRA_PEER_ID) ?: run {
                stopSelf()
                return START_NOT_STICKY
            }
            val resultCode = intent.getIntExtra(EXTRA_RESULT_CODE, 0)
            val data = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                intent.getParcelableExtra(EXTRA_RESULT_DATA, Intent::class.java)
            } else {
                @Suppress("DEPRECATION")
                intent.getParcelableExtra(EXTRA_RESULT_DATA)
            }
            if (data == null || resultCode == 0) {
                stopSession("no projection")
                return START_NOT_STICKY
            }
            val p = Peers(this).list().firstOrNull { it.id == peerId }
            if (p == null) {
                stopSession("unknown peer")
                return START_NOT_STICKY
            }
            if (running.get()) {
                teardown()
            }
            running.set(true)
            lastProblem = null
            sessionHost = intent.getStringExtra(EXTRA_HOST)
            val portExtra = intent.getIntExtra(EXTRA_PORT, -1)
            sessionPort = portExtra.takeIf { it in 1..65535 }
            val linkPeer = if (sessionHost != null && sessionPort != null) {
                p.copy(host = sessionHost!!, port = sessionPort!!)
            } else {
                p
            }
            peer = linkPeer
            activePeerName = p.name
            peersDb = Peers(this)
            startForegroundNotif(p.name, false)
            readDisplaySize()
            lastKnownDisplayW = displayW
            lastKnownDisplayH = displayH
            val peers = Peers(this)
            val lk = Link(linkPeer, peers.phoneId(), peers.phoneName(), this)
            link = lk
            connecting.set(true)
            val enc = Encoder(lk)
            encoder = enc
            startProjection(resultCode, data, enc)
            pushInputScale(enc)
            val start = Wire.Control.Start(
                width = enc.currentWidth(),
                height = enc.currentHeight(),
                fps = enc.currentFps(),
                bitrate = enc.currentBitrate(),
                codec = 1,
            )
            registerDisplayListener()
            registerNetworkCallback()
            SessionBridge.sendClipboard = { text ->
                lk.sendTcpSealed(Wire.encodeControl(Wire.Control.Clipboard(text)))
            }
            if (!isLoopbackHost(linkPeer.host)) {
                acquireWifiLock()
            }
            startStatsReporter()
            Workers.start("krapy-connect") {
                val ok = try {
                    lk.connect(start)
                } finally {
                    if (link === lk) connecting.set(false)
                }
                if (!ok && link === lk) {
                    val why = lk.lastError ?: "can't reach ${p.host}:${p.port}"
                    problem("can't connect to ${p.name}: $why")
                    mainHandler.post {
                        Toast.makeText(this, "krapy: $why", Toast.LENGTH_LONG).show()
                    }
                    beginReconnect(immediate = false)
                }
            }
            return START_NOT_STICKY
        } catch (t: Throwable) {
            val reason = t.message?.take(120) ?: t.javaClass.simpleName
            CrashLog.record(this, t, "start")
            problem("couldn't start mirroring: $reason")
            mainHandler.post {
                Toast.makeText(
                    this,
                    "krapy couldn't start mirroring: $reason",
                    Toast.LENGTH_LONG,
                ).show()
            }
            stopSession("start failed")
            return START_NOT_STICKY
        }
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        if (!running.get()) return
        try {
            onDisplayGeometryChanged()
        } catch (t: Throwable) {
            CrashLog.record(this, t, "onConfigurationChanged")
        }
    }

    override fun onControl(c: Wire.Control) {
        mainHandler.post {
            try {
                when (c) {
                    is Wire.Control.Quality -> {
                        val enc = encoder ?: return@post
                        val oldFps = enc.currentFps()
                        enc.setQuality(c.bitrate, c.fps, c.maxSize)
                        var needSurface = false
                        if (c.maxSize > 0) {
                            val (nw, nh) = enc.scaledSize(displayW, displayH, c.maxSize)
                            if (nw != enc.currentWidth() || nh != enc.currentHeight()) {
                                enc.applyMaxSize(displayW, displayH, c.maxSize)
                                needSurface = true
                            }
                        }
                        if (c.fps > 0 && c.fps != oldFps) {
                            enc.restartCodecKeepingSurface()
                            needSurface = true
                        }
                        if (needSurface) attachVirtualDisplaySurface(enc.inputSurface)
                        pushInputScale(enc)
                    }
                    Wire.Control.Keyframe -> encoder?.requestKeyframe()
                    Wire.Control.Bye -> stopSession("bye")
                    is Wire.Control.Touch -> if (InputService.InputBridge.enabled()) {
                        InputService.InputBridge.touch(c.action, c.pointer, c.x, c.y)
                    }
                    is Wire.Control.Key -> if (InputService.InputBridge.enabled()) InputService.InputBridge.key(c.code)
                    is Wire.Control.Text -> if (InputService.InputBridge.enabled()) InputService.InputBridge.text(c.text)
                    is Wire.Control.Clipboard -> applyDesktopClipboard(c.text)
                    else -> {}
                }
            } catch (t: Throwable) {
                CrashLog.record(this, t, "onControl")
            }
        }
    }

    override fun onLinkDown(reason: String) {
        problem("lost ${peer?.name ?: "desktop"}: $reason")
        mainHandler.post {
            if (!running.get()) return@post
            beginReconnect(immediate = false)
        }
    }

    override fun onDestroy() {
        stopSession("destroy")
        super.onDestroy()
    }

    private fun applyDesktopClipboard(text: String) {
        if (text == lastPhoneClipboardSent) return
        lastDesktopClipboard = text
        val cm = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        cm.setPrimaryClip(ClipData.newPlainText("krapy", text))
    }

    private fun problem(text: String) {
        lastProblem = text
        android.util.Log.w("krapy", text)
    }

    // one connect attempt at a time: the first connect or a single reconnect loop
    @Synchronized
    private fun beginReconnect(immediate: Boolean) {
        if (!running.get() || connecting.get()) return
        if (immediate) backoffMs = 250
        if (reconnectThread?.isAlive == true && reconnecting.get()) return
        link?.closeTransport()
        mainHandler.post { if (running.get()) updateNotifReconnecting() }
        reconnecting.set(true)
        reconnectThread = Workers.start("krapy-reconn") {
            reconnectLoop(immediate)
        }
    }

    private fun reconnectLoop(immediate: Boolean) {
        var delay = if (immediate) 0L else backoffMs
        while (running.get() && reconnecting.get()) {
            if (delay > 0) {
                try {
                    Thread.sleep(delay)
                } catch (_: InterruptedException) {
                    break
                }
            }
            if (!running.get() || !reconnecting.get()) break
            val p = peer ?: break
            val enc = encoder ?: break
            val lk = link ?: break
            val endpoint = PeerResolve.raceEndpoint(applicationContext, p.host, p.port, p.id)
            if (Thread.currentThread().isInterrupted) break
            if (endpoint == null) {
                problem("can't find ${p.name} on this network (last seen at ${p.host}:${p.port})")
                backoffMs = min(backoffMs * 2, 4000)
                delay = backoffMs
                continue
            }
            val (host, port) = endpoint
            val start = Wire.Control.Start(
                width = enc.currentWidth(),
                height = enc.currentHeight(),
                fps = enc.currentFps(),
                bitrate = enc.currentBitrate(),
                codec = 1,
            )
            if (lk.connect(start, host, port)) {
                lastProblem = null
                if (!isLoopbackHost(host)) {
                    peersDb?.updateEndpoint(p.id, host, port)
                }
                peer = p.copy(host = host, port = port)
                backoffMs = 250
                reconnecting.set(false)
                mainHandler.post {
                    val name = peer?.name ?: ""
                    startForegroundNotif(name, false)
                    enc.requestKeyframe()
                }
                return
            }
            lk.lastError?.let { problem("can't connect to ${p.name}: $it") }
            backoffMs = min(backoffMs * 2, 4000)
            delay = backoffMs
        }
        reconnecting.set(false)
    }

    private fun registerNetworkCallback() {
        val cm = getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                if (!running.get() || connecting.get()) return
                val lk = link ?: return
                if (!lk.isConnected()) beginReconnect(immediate = true)
            }

            override fun onCapabilitiesChanged(
                network: Network,
                caps: NetworkCapabilities,
            ) {
                if (!running.get() || connecting.get()) return
                val lk = link ?: return
                if (!lk.isConnected() && caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)) {
                    beginReconnect(immediate = true)
                }
            }
        }
        networkCallback = cb
        try {
            cm.registerDefaultNetworkCallback(cb, mainHandler)
        } catch (_: Exception) {
        }
    }

    private fun pushInputScale(enc: Encoder) {
        InputService.InputBridge.setVideoSize(enc.currentWidth(), enc.currentHeight(), displayW, displayH)
    }

    private fun startProjection(resultCode: Int, data: Intent, enc: Encoder) {
        val mgr = getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        val proj = try {
            mgr.getMediaProjection(resultCode, data)
        } catch (e: SecurityException) {
            throw IllegalStateException("projection: ${e.message}")
        } ?: throw IllegalStateException("projection denied")
        if (Build.VERSION.SDK_INT >= 34) {
            val cb = object : MediaProjection.Callback() {
                override fun onStop() {
                    mainHandler.post { stopSession("screen capture stopped") }
                }
            }
            projectionCallback = cb
            proj.registerCallback(cb, mainHandler)
        }
        projection = proj
        enc.start(displayW, displayH, 1920, 60, 12_000_000)
        val surface = enc.inputSurface
        try {
            virtualDisplay = createDisplay(proj, surface, enc.currentWidth(), enc.currentHeight())
        } catch (e: SecurityException) {
            throw IllegalStateException("virtual display: ${e.message}")
        }
    }

    private fun createDisplay(
        proj: MediaProjection,
        surface: Surface?,
        w: Int,
        h: Int,
    ): VirtualDisplay? {
        if (surface == null) return null
        val dpi = resources.displayMetrics.densityDpi
        val flags = DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR
        return proj.createVirtualDisplay(
            "krapy",
            w,
            h,
            dpi,
            flags,
            surface,
            null,
            mainHandler,
        )
    }

    private fun attachVirtualDisplaySurface(surface: Surface?) {
        val vd = virtualDisplay ?: return
        val s = surface ?: return
        val w = encoder?.currentWidth() ?: displayW
        val h = encoder?.currentHeight() ?: displayH
        val dpi = resources.displayMetrics.densityDpi
        vd.resize(w, h, dpi)
        vd.setSurface(s)
    }

    private fun onDisplayGeometryChanged() {
        val prevW = lastKnownDisplayW
        val prevH = lastKnownDisplayH
        readDisplaySize()
        if (displayW == prevW && displayH == prevH) return
        lastKnownDisplayW = displayW
        lastKnownDisplayH = displayH
        val enc = encoder ?: return
        enc.restart(displayW, displayH, null)
        attachVirtualDisplaySurface(enc.inputSurface)
        pushInputScale(enc)
    }

    private fun registerDisplayListener() {
        val dm = getSystemService(Context.DISPLAY_SERVICE) as DisplayManager
        val listener = object : DisplayManager.DisplayListener {
            override fun onDisplayAdded(displayId: Int) {}
            override fun onDisplayRemoved(displayId: Int) {}
            override fun onDisplayChanged(displayId: Int) {
                if (displayId != Display.DEFAULT_DISPLAY) return
                if (!running.get()) return
                try {
                    onDisplayGeometryChanged()
                } catch (t: Throwable) {
                    CrashLog.record(this@MirrorService, t, "onDisplayChanged")
                }
            }
        }
        displayListener = listener
        dm.registerDisplayListener(listener, mainHandler)
    }

    private fun readDisplaySize() {
        val wm = getSystemService(Context.WINDOW_SERVICE) as WindowManager
        val metrics = DisplayMetrics()
        @Suppress("DEPRECATION")
        wm.defaultDisplay.getRealMetrics(metrics)
        displayW = metrics.widthPixels
        displayH = metrics.heightPixels
    }

    private fun stopSession(reason: String) {
        if (!running.get()) {
            return
        }
        teardown()
        updateNotifStopped(reason)
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    private fun acquireWifiLock() {
        val wm = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        val mode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            WifiManager.WIFI_MODE_FULL_LOW_LATENCY
        } else {
            @Suppress("DEPRECATION")
            WifiManager.WIFI_MODE_FULL_HIGH_PERF
        }
        wifiLock = wm.createWifiLock(mode, "krapy-wifi").also { it.acquire() }
    }

    private fun releaseWifiLock() {
        try {
            wifiLock?.release()
        } catch (_: Exception) {
        }
        wifiLock = null
    }

    private fun startStatsReporter() {
        statsThread = Workers.start("krapy-stats") {
            while (running.get()) {
                try {
                    Thread.sleep(1000)
                } catch (_: InterruptedException) {
                    break
                }
                if (!running.get()) break
                val enc = encoder ?: continue
                val lk = link ?: continue
                enc.drainEncoderStatsWindow()
                val (avg, max, fps) = enc.encoderStats()
                lk.sendPhoneStats(avg, max, fps, enc.currentBitrate())
            }
        }
    }

    private fun teardown() {
        statsThread?.interrupt()
        statsThread = null
        releaseWifiLock()
        running.set(false)
        connecting.set(false)
        activePeerName = null
        reconnecting.set(false)
        reconnectThread?.interrupt()
        reconnectThread = null
        SessionBridge.sendClipboard = null
        networkCallback?.let {
            val cm = getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
            try {
                cm.unregisterNetworkCallback(it)
            } catch (_: Exception) {
            }
        }
        networkCallback = null
        displayListener?.let {
            val dm = getSystemService(Context.DISPLAY_SERVICE) as DisplayManager
            dm.unregisterDisplayListener(it)
        }
        displayListener = null
        encoder?.stop()
        encoder = null
        link?.close()
        link = null
        virtualDisplay?.release()
        virtualDisplay = null
        projectionCallback?.let { projection?.unregisterCallback(it) }
        projectionCallback = null
        projection?.stop()
        projection = null
    }

    private fun startForegroundNotif(name: String, reconnect: Boolean) {
        ensureChannel()
        val stopPi = PendingIntent.getService(
            this,
            0,
            Intent(this, MirrorService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val text = if (reconnect) "reconnecting" else "mirroring $name"
        val notif = Notification.Builder(this, CHANNEL_ID)
            .setContentTitle("krapy")
            .setContentText(text)
            .setSmallIcon(android.R.drawable.ic_menu_view)
            .addAction(
                Notification.Action.Builder(
                    null,
                    "Stop",
                    stopPi,
                ).build(),
            )
            .setOngoing(true)
            .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(
                NOTIF_ID,
                notif,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION,
            )
        } else {
            startForeground(NOTIF_ID, notif)
        }
    }

    private fun updateNotifReconnecting() {
        val name = peer?.name ?: ""
        startForegroundNotif(name, true)
    }

    private fun updateNotifStopped(reason: String) {
        val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        ensureChannel()
        val notif = Notification.Builder(this, CHANNEL_ID)
            .setContentTitle("krapy")
            .setContentText("session ended: $reason")
            .setSmallIcon(android.R.drawable.ic_menu_view)
            .build()
        nm.notify(NOTIF_ID + 1, notif)
    }

    private fun ensureChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (nm.getNotificationChannel(CHANNEL_ID) == null) {
            nm.createNotificationChannel(
                NotificationChannel(CHANNEL_ID, "krapy", NotificationManager.IMPORTANCE_LOW),
            )
        }
    }

    companion object {
        const val ACTION_STOP = "com.waghitech.krapy.STOP"
        const val EXTRA_PEER_ID = "peer_id"
        const val EXTRA_RESULT_CODE = "result_code"
        const val EXTRA_RESULT_DATA = "result_data"
        const val EXTRA_HOST = "host"
        const val EXTRA_PORT = "port"
        private const val CHANNEL_ID = "krapy_mirror"
        private const val NOTIF_ID = 1

        @Volatile
        var lastDesktopClipboard: String? = null

        @Volatile
        var lastPhoneClipboardSent: String? = null

        @Volatile
        var activePeerName: String? = null

        @Volatile
        var lastProblem: String? = null

        private fun isLoopbackHost(host: String): Boolean =
            host == "127.0.0.1" || host.equals("localhost", ignoreCase = true)

        fun start(
            context: Context,
            peerId: String,
            resultCode: Int,
            data: Intent,
            hostOverride: String? = null,
            portOverride: Int? = null,
        ) {
            val i = Intent(context, MirrorService::class.java).apply {
                putExtra(EXTRA_PEER_ID, peerId)
                putExtra(EXTRA_RESULT_CODE, resultCode)
                putExtra(EXTRA_RESULT_DATA, data)
                if (hostOverride != null && portOverride != null) {
                    putExtra(EXTRA_HOST, hostOverride)
                    putExtra(EXTRA_PORT, portOverride)
                }
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(i)
            } else {
                context.startService(i)
            }
        }
    }
}