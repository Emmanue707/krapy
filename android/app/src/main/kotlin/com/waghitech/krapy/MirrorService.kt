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
import android.net.NetworkRequest
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
import kotlin.concurrent.thread
import kotlin.math.min

class MirrorService : Service(), LinkCallbacks {
    private val running = AtomicBoolean(false)
    private val reconnecting = AtomicBoolean(false)
    private var projection: MediaProjection? = null
    private var virtualDisplay: VirtualDisplay? = null
    private var encoder: Encoder? = null
    private var link: Link? = null
    private var peer: Peer? = null
    private var peersDb: Peers? = null
    private var displayW: Int = 0
    private var displayH: Int = 0
    private var captureBaseNs: Long = 0
    private val mainHandler = Handler(Looper.getMainLooper())
    private var projectionCallback: MediaProjection.Callback? = null
    private var displayListener: DisplayManager.DisplayListener? = null
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private var reconnectThread: Thread? = null
    @Volatile
    private var backoffMs: Long = 250

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopSession("stopped")
            return START_NOT_STICKY
        }
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
        if (running.getAndSet(true)) {
            stopSession("restarted")
        }
        peer = p
        activePeerName = p.name
        peersDb = Peers(this)
        startForegroundNotif(p.name, false)
        captureBaseNs = System.nanoTime()
        readDisplaySize()
        val peers = Peers(this)
        val lk = Link(p, peers.phoneId(), peers.phoneName(), this)
        link = lk
        val enc = Encoder(lk, captureBaseNs)
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
        if (!lk.connect(start)) {
            mainHandler.post {
                Toast.makeText(
                    this,
                    "can't reach ${p.host}:${p.port}. Same Wi-Fi? On the PC run krapy doctor (firewall)",
                    Toast.LENGTH_LONG,
                ).show()
            }
            beginReconnect(immediate = false)
        } else {
            reconnecting.set(false)
        }
        return START_STICKY
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        if (!running.get()) return
        readDisplaySize()
        val enc = encoder ?: return
        enc.restart(displayW, displayH, null)
        resetVirtualDisplay(enc.inputSurface)
        pushInputScale(enc)
    }

    override fun onControl(c: Wire.Control) {
        mainHandler.post {
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
                    if (needSurface) resetVirtualDisplay(enc.inputSurface)
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
        }
    }

    override fun onLinkDown(reason: String) {
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

    private fun beginReconnect(immediate: Boolean) {
        if (!running.get()) return
        if (immediate) backoffMs = 250
        link?.closeTransport()
        updateNotifReconnecting()
        reconnecting.set(true)
        reconnectThread?.interrupt()
        reconnectThread = thread(name = "krapy-reconn", isDaemon = true) {
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
            if (endpoint == null) {
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
                peersDb?.updateEndpoint(p.id, host, port)
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
            backoffMs = min(backoffMs * 2, 4000)
            delay = backoffMs
        }
        reconnecting.set(false)
    }

    private fun registerNetworkCallback() {
        val cm = getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                if (!running.get()) return
                val lk = link ?: return
                if (!lk.isConnected()) beginReconnect(immediate = true)
            }

            override fun onCapabilitiesChanged(
                network: Network,
                caps: NetworkCapabilities,
            ) {
                if (!running.get()) return
                val lk = link ?: return
                if (!lk.isConnected() && caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)) {
                    beginReconnect(immediate = true)
                }
            }
        }
        networkCallback = cb
        try {
            cm.registerNetworkCallback(NetworkRequest.Builder().build(), cb)
        } catch (_: Exception) {
        }
    }

    private fun pushInputScale(enc: Encoder) {
        InputService.InputBridge.setVideoSize(enc.currentWidth(), enc.currentHeight(), displayW, displayH)
    }

    private fun startProjection(resultCode: Int, data: Intent, enc: Encoder) {
        val mgr = getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        val proj = mgr.getMediaProjection(resultCode, data) ?: run {
            stopSession("projection denied")
            return
        }
        if (Build.VERSION.SDK_INT >= 34) {
            val cb = object : MediaProjection.Callback() {}
            projectionCallback = cb
            proj.registerCallback(cb, mainHandler)
        }
        projection = proj
        enc.start(displayW, displayH, 1280, 30, 4_000_000)
        val surface = enc.inputSurface
        virtualDisplay = createDisplay(proj, surface, enc.currentWidth(), enc.currentHeight())
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

    private fun resetVirtualDisplay(surface: Surface?) {
        val vd = virtualDisplay
        vd?.release()
        val proj = projection ?: return
        virtualDisplay = createDisplay(proj, surface, encoder?.currentWidth() ?: displayW, encoder?.currentHeight() ?: displayH)
    }

    private fun registerDisplayListener() {
        val dm = getSystemService(Context.DISPLAY_SERVICE) as DisplayManager
        val listener = object : DisplayManager.DisplayListener {
            override fun onDisplayAdded(displayId: Int) {}
            override fun onDisplayRemoved(displayId: Int) {}
            override fun onDisplayChanged(displayId: Int) {
                if (displayId != Display.DEFAULT_DISPLAY) return
                if (!running.get()) return
                readDisplaySize()
                val enc = encoder ?: return
                enc.restart(displayW, displayH, null)
                resetVirtualDisplay(enc.inputSurface)
                pushInputScale(enc)
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
        if (!running.getAndSet(false)) {
            return
        }
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
        updateNotifStopped(reason)
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
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
        private const val CHANNEL_ID = "krapy_mirror"
        private const val NOTIF_ID = 1

        @Volatile
        var lastDesktopClipboard: String? = null

        @Volatile
        var lastPhoneClipboardSent: String? = null

        @Volatile
        var activePeerName: String? = null

        fun start(context: Context, peerId: String, resultCode: Int, data: Intent) {
            val i = Intent(context, MirrorService::class.java).apply {
                putExtra(EXTRA_PEER_ID, peerId)
                putExtra(EXTRA_RESULT_CODE, resultCode)
                putExtra(EXTRA_RESULT_DATA, data)
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(i)
            } else {
                context.startService(i)
            }
        }
    }
}