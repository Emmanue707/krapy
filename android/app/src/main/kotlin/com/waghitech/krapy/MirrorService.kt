package com.waghitech.krapy

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.widget.RemoteViews
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.content.res.Configuration
import android.Manifest
import android.content.pm.PackageManager
import android.media.AudioManager
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.net.ConnectivityManager
import android.os.PowerManager
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
    private var fgTypes = -1
    private var micWanted = false
    private var statsThread: Thread? = null
    private var audioCapture: AudioCapture? = null
    private var wakeLock: PowerManager.WakeLock? = null
    @Volatile
    private var paused = false
    @Volatile
    private var audioOnPc = false
    @Volatile
    private var pcRotation = 0
    private var transportLabel = "Wi‑Fi"
    private var lastNotifSnap: SessionState.Snap? = null
    private var lastNotifAtMs = 0L

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        restoreSavedMusicVolume()
    }

    private fun restoreSavedMusicVolume() {
        val v = SessionPrefs.takeSavedMusicVolume(this) ?: return
        val am = getSystemService(Context.AUDIO_SERVICE) as AudioManager
        am.setStreamVolume(AudioManager.STREAM_MUSIC, v, 0)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        restoreSavedMusicVolume()
        when (intent?.action) {
            ACTION_STOP, ACTION_DISCONNECT -> {
                stopSession("stopped")
                return START_NOT_STICKY
            }
            ACTION_TOGGLE_PAUSE -> {
                if (running.get()) togglePause()
                return START_STICKY
            }
            ACTION_TOGGLE_AUDIO -> {
                if (running.get()) toggleAudioFromNotif()
                return START_STICKY
            }
            ACTION_ROTATE -> {
                if (running.get()) rotatePcViewFromPhone()
                return START_STICKY
            }
            ACTION_SET_PRESET -> {
                val id = intent.getIntExtra(EXTRA_PRESET, SessionPrefs.PRESET_BALANCED)
                applyPreset(id)
                return START_STICKY
            }
            ACTION_PAUSE_FROM_PC -> {
                val on = intent.getIntExtra(EXTRA_VALUE, 0) != 0
                if (running.get()) setPaused(on)
                return START_STICKY
            }
            ACTION_PRESET_FROM_PC -> {
                val id = intent.getIntExtra(EXTRA_VALUE, SessionPrefs.PRESET_BALANCED)
                if (running.get()) applyPreset(id)
                return START_STICKY
            }
            ACTION_SOUND_PC -> {
                val on = intent.getBooleanExtra(EXTRA_BOOL, false)
                if (running.get()) setSoundOnPc(on)
                return START_STICKY
            }
            ACTION_CLIPBOARD -> {
                val on = intent.getBooleanExtra(EXTRA_BOOL, false)
                setClipboardSync(on)
                return START_STICKY
            }
            ACTION_WAKE -> {
                val on = intent.getBooleanExtra(EXTRA_BOOL, false)
                setKeepAwake(on)
                return START_STICKY
            }
            null -> { }
            else -> return START_STICKY
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
            transportLabel = if (isLoopbackHost(linkPeer.host)) "USB" else "Wi‑Fi"
            paused = false
            SessionState.mirroring = true
            SessionState.audioOnPc = SessionPrefs.soundOnPc(this)
            audioOnPc = SessionState.audioOnPc
            SessionState.clipboardSync = SessionPrefs.clipboardSync(this)
            SessionBridge.clipboardSync = SessionState.clipboardSync
            peersDb = Peers(this)
            if (SessionPrefs.keepAwake(this)) acquireScreenWake()
            refreshForegroundNotif(false)
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
                if (SessionBridge.clipboardSync) {
                    lk.sendTcpSealed(Wire.encodeControl(Wire.Control.Clipboard(text)))
                }
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
                if (ok && link === lk) {
                    lk.sendPreset(SessionPrefs.preset(this@MirrorService))
                    pushSettings()
                    if (SessionPrefs.soundOnPc(this@MirrorService)) {
                        startAudioCapture()
                    }
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
                    is Wire.Control.SetSetting -> mainHandler.post { applySettingFromPc(c.key, c.value) }
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
        if (!SessionBridge.clipboardSync) return
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
        mainHandler.post { if (running.get()) refreshForegroundNotif(true) }
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
                    refreshForegroundNotif(false)
                    enc.requestKeyframe()
                }
                lk.sendPreset(SessionPrefs.preset(this@MirrorService))
                pushSettings()
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
        val pq = SessionPrefs.presetQuality(SessionPrefs.preset(this))
        enc.start(displayW, displayH, pq.maxLong, pq.fps, pq.bitrate)
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
                if (enc.needsBaselineRestart()) {
                    mainHandler.post {
                        enc.restartCodecKeepingSurface()
                        attachVirtualDisplaySurface(enc.inputSurface)
                    }
                }
                val (avg, max, fps) = enc.encoderStats()
                val br = enc.currentBitrate()
                lk.sendPhoneStats(avg, max, fps, br)
                val ds = lk.latestDesktopStats
                val snap = SessionState.Snap(
                    peerName = peer?.name ?: "",
                    transport = transportLabel,
                    fps = ds?.fps ?: fps,
                    latencyMs = ds?.latencyMs ?: 0,
                    bitrateBps = br,
                    paused = paused,
                )
                SessionState.snap = snap
                val now = System.currentTimeMillis()
                val due = now - lastNotifAtMs >= 2000
                val changed = snap != lastNotifSnap
                if (due && changed) {
                    mainHandler.post { refreshForegroundNotif(false) }
                }
            }
        }
    }

    private fun teardown() {
        stopAudioCapture()
        fgTypes = -1
        releaseScreenWake()
        statsThread?.interrupt()
        statsThread = null
        releaseWifiLock()
        running.set(false)
        connecting.set(false)
        activePeerName = null
        SessionState.mirroring = false
        SessionState.snap = SessionState.Snap()
        SessionState.audioOnPc = false
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

    private fun refreshForegroundNotif(reconnect: Boolean) {
        val snap = SessionState.snap
        val name = snap.peerName.ifEmpty { peer?.name ?: "desktop" }
        lastNotifSnap = if (reconnect) null else snap
        lastNotifAtMs = System.currentTimeMillis()
        val notif = buildSessionNotification(
            context = this,
            peerName = name,
            stats = if (reconnect) "" else NotifFormat.contentText(snap),
            paused = paused,
            audioOnPc = audioOnPc,
            reconnect = reconnect,
        )
        val types = foregroundServiceTypes()
        if (types == fgTypes) {
            (getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager).notify(NOTIF_ID, notif)
            return
        }
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                startForeground(NOTIF_ID, notif, types)
            } else {
                startForeground(NOTIF_ID, notif)
            }
            fgTypes = types
        } catch (e: Exception) {
            if (micWanted) {
                micWanted = false
                audioOnPc = false
                SessionState.audioOnPc = false
                lastProblem = "open krapy to send sound to the PC"
                refreshForegroundNotif(reconnect)
            }
        }
    }

    private fun foregroundServiceTypes(): Int {
        var t = ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION
        if (Build.VERSION.SDK_INT >= 34 && micWanted) {
            t = t or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
        }
        return t
    }

    private fun togglePause() {
        setPaused(!paused)
    }

    private fun setPaused(on: Boolean) {
        val enc = encoder ?: return
        val lk = link ?: return
        if (paused == on) return
        if (on) {
            virtualDisplay?.setSurface(null)
            paused = true
            lk.sendPaused(true)
        } else {
            attachVirtualDisplaySurface(enc.inputSurface)
            paused = false
            enc.requestKeyframe()
            lk.sendPaused(false)
        }
        SessionState.snap = SessionState.snap.copy(paused = paused)
        refreshForegroundNotif(false)
        pushSettings()
    }

    private fun applyPreset(id: Int) {
        val preset = id.coerceIn(SessionPrefs.PRESET_SMOOTH, SessionPrefs.PRESET_SHARP)
        SessionPrefs.prefs(this).edit().putInt(SessionPrefs.PREF_PRESET, preset).apply()
        link?.sendPreset(preset)
        val pq = SessionPrefs.presetQuality(preset)
        encoder?.setQuality(pq.bitrate, pq.fps, pq.maxLong)
        encoder?.let { enc ->
            val (nw, nh) = enc.scaledSize(displayW, displayH, pq.maxLong)
            if (nw != enc.currentWidth() || nh != enc.currentHeight()) {
                enc.applyMaxSize(displayW, displayH, pq.maxLong)
                attachVirtualDisplaySurface(enc.inputSurface)
            }
            if (enc.needsBaselineRestart()) {
                enc.restartCodecKeepingSurface()
                attachVirtualDisplaySurface(enc.inputSurface)
            }
            pushInputScale(enc)
        }
        pushSettings()
    }

    private fun setClipboardSync(on: Boolean) {
        SessionBridge.clipboardSync = on
        SessionState.clipboardSync = on
        SessionPrefs.prefs(this).edit().putBoolean(SessionPrefs.PREF_CLIPBOARD, on).apply()
        pushSettings()
    }

    private fun setKeepAwake(on: Boolean) {
        SessionPrefs.prefs(this).edit().putBoolean(SessionPrefs.PREF_KEEP_AWAKE, on).apply()
        if (on) acquireScreenWake() else releaseScreenWake()
        pushSettings()
    }

    private fun rotatePcViewFromPhone() {
        pcRotation = (pcRotation + 1) and 3
        pushSettings()
    }

    private fun pushSettings() {
        val lk = link ?: return
        lk.sendSettings(
            paused,
            SessionPrefs.preset(this),
            audioOnPc,
            SessionPrefs.keepAwake(this),
            SessionBridge.clipboardSync,
            pcRotation,
        )
    }

    fun applySettingFromPc(key: Int, value: Int) {
        if (!running.get()) return
        val keepAwake = SessionPrefs.keepAwake(this)
        val clip = SessionBridge.clipboardSync
        val v = PcCommands.normalizedValue(key, value, paused, audioOnPc, keepAwake, clip)
        when (key) {
            PcCommands.KEY_PAUSE -> setPaused(v != 0)
            PcCommands.KEY_PRESET -> applyPreset(v)
            PcCommands.KEY_SOUND_PC -> setSoundOnPc(v != 0)
            PcCommands.KEY_KEEP_AWAKE -> setKeepAwake(v != 0)
            PcCommands.KEY_CLIPBOARD -> setClipboardSync(v != 0)
            PcCommands.KEY_ROTATE -> {
                pcRotation = (pcRotation + 1) and 3
                pushSettings()
            }
            PcCommands.KEY_DISCONNECT -> stopSession("desktop disconnect")
        }
    }

    private fun toggleAudioFromNotif() {
        setSoundOnPc(!audioOnPc)
    }

    fun setSoundOnPc(on: Boolean) {
        audioOnPc = on
        SessionState.audioOnPc = on
        SessionPrefs.prefs(this).edit().putBoolean(SessionPrefs.PREF_SOUND_PC, on).apply()
        if (on) {
            startAudioCapture()
        } else {
            stopAudioCapture()
        }
        refreshForegroundNotif(false)
        pushSettings()
    }

    private fun startAudioCapture() {
        if (audioCapture != null) return
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            return
        }
        val proj = projection ?: return
        val lk = link ?: return
        val am = getSystemService(Context.AUDIO_SERVICE) as AudioManager
        micWanted = true
        refreshForegroundNotif(false)
        if (!micWanted) return
        audioCapture = try {
            AudioCapture(proj, lk, am, this) {
                mainHandler.post {
                    lastProblem = "this phone keeps playing sound while sending it to the PC"
                    refreshForegroundNotif(false)
                }
            }.also { it.start() }
        } catch (t: Throwable) {
            CrashLog.record(this, t, "audio")
            micWanted = false
            audioOnPc = false
            SessionState.audioOnPc = false
            lastProblem = "sound capture failed on this phone"
            null
        }
        refreshForegroundNotif(false)
    }

    private fun stopAudioCapture() {
        audioCapture?.stop()
        audioCapture = null
        micWanted = false
        refreshForegroundNotif(false)
    }

    private fun acquireScreenWake() {
        if (wakeLock?.isHeld == true) return
        val pm = getSystemService(Context.POWER_SERVICE) as PowerManager
        @Suppress("DEPRECATION")
        val wl = pm.newWakeLock(
            PowerManager.SCREEN_BRIGHT_WAKE_LOCK or PowerManager.ACQUIRE_CAUSES_WAKEUP,
            "krapy:screen",
        )
        wl.setReferenceCounted(false)
        wl.acquire()
        wakeLock = wl
    }

    private fun releaseScreenWake() {
        try {
            wakeLock?.let { if (it.isHeld) it.release() }
        } catch (_: Exception) {
        }
        wakeLock = null
    }

    private fun updateNotifStopped(reason: String) {
        val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        ensureChannel()
        val notif = Notification.Builder(this, SESSION_CHANNEL_ID)
            .setContentTitle("krapy")
            .setContentText("session ended: $reason")
            .setSmallIcon(android.R.drawable.ic_menu_view)
            .build()
        nm.notify(NOTIF_ID + 1, notif)
    }

    private fun ensureChannel() {
        ensureSessionChannel(this)
    }

    companion object {
        const val ACTION_STOP = "com.waghitech.krapy.STOP"
        const val ACTION_DISCONNECT = "com.waghitech.krapy.DISCONNECT"
        const val ACTION_TOGGLE_PAUSE = "com.waghitech.krapy.TOGGLE_PAUSE"
        const val ACTION_TOGGLE_AUDIO = "com.waghitech.krapy.TOGGLE_AUDIO"
        const val ACTION_ROTATE = "com.waghitech.krapy.ROTATE"
        const val ACTION_SET_PRESET = "com.waghitech.krapy.SET_PRESET"
        const val ACTION_SOUND_PC = "com.waghitech.krapy.SOUND_PC"
        const val ACTION_CLIPBOARD = "com.waghitech.krapy.CLIPBOARD"
        const val ACTION_WAKE = "com.waghitech.krapy.WAKE"
        const val ACTION_PAUSE_FROM_PC = "com.waghitech.krapy.PAUSE_FROM_PC"
        const val ACTION_PRESET_FROM_PC = "com.waghitech.krapy.PRESET_FROM_PC"
        const val EXTRA_PRESET = "preset"
        const val EXTRA_BOOL = "on"
        const val EXTRA_VALUE = "value"
        const val EXTRA_PEER_ID = "peer_id"
        const val EXTRA_RESULT_CODE = "result_code"
        const val EXTRA_RESULT_DATA = "result_data"
        const val EXTRA_HOST = "host"
        const val EXTRA_PORT = "port"
        internal const val SESSION_CHANNEL_ID = "krapy_session"
        private const val LEGACY_CHANNEL_ID = "krapy_mirror"
        private const val NOTIF_ID = 1

        fun sessionNotificationsVisible(context: Context): Boolean {
            val nm = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            if (!nm.areNotificationsEnabled()) return false
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return true
            val ch = nm.getNotificationChannel(SESSION_CHANNEL_ID) ?: return true
            return ch.importance != NotificationManager.IMPORTANCE_NONE
        }

        fun ensureSessionChannel(context: Context) {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
            val nm = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            if (nm.getNotificationChannel(SESSION_CHANNEL_ID) != null) return
            nm.deleteNotificationChannel(LEGACY_CHANNEL_ID)
            val ch = NotificationChannel(
                SESSION_CHANNEL_ID,
                "krapy session",
                NotificationManager.IMPORTANCE_DEFAULT,
            ).apply {
                setSound(null, null)
                enableVibration(false)
                setShowBadge(false)
                lockscreenVisibility = Notification.VISIBILITY_PUBLIC
            }
            nm.createNotificationChannel(ch)
        }

        internal fun buildSessionNotification(
            context: Context,
            peerName: String,
            stats: String,
            paused: Boolean,
            audioOnPc: Boolean,
            reconnect: Boolean,
        ): Notification {
            ensureSessionChannel(context)
            val openApp = PendingIntent.getActivity(
                context,
                2,
                Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            val pausePi = PendingIntent.getService(
                context,
                3,
                Intent(context, MirrorService::class.java).setAction(ACTION_TOGGLE_PAUSE),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            val soundPi = PendingIntent.getService(
                context,
                4,
                Intent(context, MirrorService::class.java).setAction(ACTION_TOGGLE_AUDIO),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            val stopPi = PendingIntent.getService(
                context,
                0,
                Intent(context, MirrorService::class.java).setAction(ACTION_DISCONNECT),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            val views = RemoteViews(context.packageName, R.layout.notif_session).apply {
                setTextViewText(R.id.notif_title, NotifFormat.sessionTitle(peerName, reconnect))
                setTextViewText(
                    R.id.notif_stats,
                    if (reconnect) "…" else stats,
                )
                setTextViewText(R.id.notif_btn_pause, NotifFormat.pauseActionLabel(paused))
                setTextViewText(R.id.notif_btn_sound, NotifFormat.soundActionLabel(audioOnPc))
                setTextViewText(R.id.notif_btn_disconnect, "Disconnect")
                setOnClickPendingIntent(R.id.notif_btn_pause, pausePi)
                setOnClickPendingIntent(R.id.notif_btn_sound, soundPi)
                setOnClickPendingIntent(R.id.notif_btn_disconnect, stopPi)
            }
            val builder = Notification.Builder(context, SESSION_CHANNEL_ID)
                .setSmallIcon(android.R.drawable.ic_menu_view)
                .setContentIntent(openApp)
                .setCustomContentView(views)
                .setCustomBigContentView(views)
                .setStyle(Notification.DecoratedCustomViewStyle())
                .setOnlyAlertOnce(true)
                .setCategory(Notification.CATEGORY_SERVICE)
                .setOngoing(true)
                .addAction(
                    Notification.Action.Builder(
                        null,
                        NotifFormat.pauseActionLabel(paused),
                        pausePi,
                    ).build(),
                )
                .addAction(
                    Notification.Action.Builder(
                        null,
                        NotifFormat.soundActionLabel(audioOnPc),
                        soundPi,
                    ).build(),
                )
                .addAction(
                    Notification.Action.Builder(
                        null,
                        "Disconnect",
                        stopPi,
                    ).build(),
                )
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                builder.setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE)
            }
            return builder.build()
        }

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