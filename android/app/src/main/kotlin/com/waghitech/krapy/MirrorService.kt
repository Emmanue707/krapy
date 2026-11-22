package com.waghitech.krapy

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.content.res.Configuration
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.DisplayMetrics
import android.view.Display
import android.view.Surface
import android.view.WindowManager
import java.util.concurrent.atomic.AtomicBoolean

class MirrorService : Service(), LinkCallbacks {
    private val running = AtomicBoolean(false)
    private var projection: MediaProjection? = null
    private var virtualDisplay: VirtualDisplay? = null
    private var encoder: Encoder? = null
    private var link: Link? = null
    private var peer: Peer? = null
    private var displayW: Int = 0
    private var displayH: Int = 0
    private var captureBaseNs: Long = 0
    private val mainHandler = Handler(Looper.getMainLooper())
    private var projectionCallback: MediaProjection.Callback? = null
    private var displayListener: DisplayManager.DisplayListener? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopSession("stopped")
            return START_NOT_STICKY
        }
        run {
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
                startForegroundNotif(p.name)
                captureBaseNs = System.nanoTime()
                val peers = Peers(this)
                val lk = Link(p, peers.phoneId(), peers.phoneName(), this)
                link = lk
                val enc = Encoder(lk, captureBaseNs)
                encoder = enc
                readDisplaySize()
                startProjection(resultCode, data, enc)
                val start = Wire.Control.Start(
                    width = enc.currentWidth(),
                    height = enc.currentHeight(),
                    fps = enc.currentFps(),
                    bitrate = enc.currentBitrate(),
                    codec = 1,
                )
                if (!lk.connect(start)) {
                    return START_NOT_STICKY
                }
                registerDisplayListener()
                return START_STICKY
        }
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        if (!running.get()) return
        readDisplaySize()
        val enc = encoder ?: return
        enc.restart(displayW, displayH, null)
        resetVirtualDisplay(enc.inputSurface)
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
                }
                Wire.Control.Keyframe -> encoder?.requestKeyframe()
                Wire.Control.Bye -> stopSession("bye")
                is Wire.Control.Touch, is Wire.Control.Key, is Wire.Control.Text,
                is Wire.Control.Clipboard,
                -> {
                }
                else -> {}
            }
        }
    }

    override fun onLost(reason: String) {
        mainHandler.post { stopSession(reason) }
    }

    override fun onDestroy() {
        stopSession("destroy")
        super.onDestroy()
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
        var flags = DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR
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

    private fun startForegroundNotif(name: String) {
        ensureChannel()
        val stopPi = PendingIntent.getService(
            this,
            0,
            Intent(this, MirrorService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val notif = Notification.Builder(this, CHANNEL_ID)
            .setContentTitle("krapy")
            .setContentText("mirroring $name")
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