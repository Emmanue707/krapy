package com.waghitech.krapy

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.res.ColorStateList
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.text.method.ScrollingMovementMethod
import android.media.projection.MediaProjectionManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.util.TypedValue
import android.view.View
import android.view.ViewGroup
import android.view.WindowInsets
import android.widget.AdapterView
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ListView
import android.widget.TextView
import android.widget.Toast
import kotlin.math.max

class MainActivity : Activity() {
    private lateinit var peers: Peers
    private lateinit var listView: ListView
    private lateinit var uriField: EditText
    private lateinit var statusLine: TextView
    private lateinit var remoteBlock: LinearLayout
    private lateinit var pairedLabel: TextView
    private lateinit var emptyPeers: TextView
    private var peerRows: List<Peer> = emptyList()
    private var peerAdapter: PeerAdapter? = null
    private var pendingPeer: Peer? = null
    private var pendingHostOverride: String? = null
    private var pendingPortOverride: Int? = null
    private var remoteHintText = "remote control needs accessibility (optional)"
    private val basePadH by lazy { dp(20) }
    private val basePadTop by lazy { dp(16) }
    private val basePadBottom by lazy { dp(20) }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        peers = Peers(this)
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(basePadH, basePadTop, basePadH, basePadBottom)
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
        }
        applyWindowInsets(root)
        root.addView(TextView(this).apply {
            text = "krapy"
            textSize = 28f
            setTypeface(typeface, Typeface.BOLD)
            setTextColor(textPrimary())
        })
        root.addView(TextView(this).apply {
            text = "${peers.phoneName()} · ${peers.phoneId()}"
            textSize = 13f
            setTextColor(textSecondary())
            setPadding(0, dp(4), 0, dp(12))
        })
        statusLine = TextView(this).apply {
            textSize = 14f
            setTextColor(textSecondary())
            setPadding(0, 0, 0, dp(16))
        }
        root.addView(statusLine)
        remoteBlock = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            visibility = View.GONE
        }
        root.addView(remoteBlock)
        pairedLabel = TextView(this).apply {
            text = "paired desktops · tap to mirror, hold to forget"
            textSize = 12f
            setTypeface(typeface, Typeface.BOLD)
            setTextColor(textSecondary())
            setPadding(0, dp(8), 0, dp(4))
        }
        root.addView(pairedLabel)
        emptyPeers = TextView(this).apply {
            text = "no desktops paired yet"
            textSize = 13f
            setTextColor(textSecondary())
            setPadding(0, dp(4), 0, dp(8))
            visibility = View.GONE
        }
        root.addView(emptyPeers)
        listView = ListView(this).apply {
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                0,
                1f,
            )
        }
        root.addView(listView)
        root.addView(primaryButton("Scan QR") { startScan() }.apply {
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
            ).apply { topMargin = dp(12) }
        })
        uriField = EditText(this).apply {
            hint = "krapy://pair?..."
            setSingleLine()
            minHeight = dp(48)
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
            ).apply { topMargin = dp(12) }
        }
        root.addView(uriField)
        root.addView(primaryButton("Pair") { pairFromField() }.apply {
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
            ).apply { topMargin = dp(8) }
        })
        setContentView(root)
        maybeShowLastCrash()
        refreshRemoteBlock()
        refreshStatus()
        listView.onItemClickListener = AdapterView.OnItemClickListener { _, _, pos, _ ->
            startMirroring(peerRows[pos])
        }
        listView.onItemLongClickListener = AdapterView.OnItemLongClickListener { _, _, pos, _ ->
            confirmForget(peerRows[pos])
            true
        }
        refreshList()
        handleViewIntent(intent)
        maybeRequestNotif()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleViewIntent(intent)
    }

    override fun onResume() {
        super.onResume()
        val prefs = uiPrefs()
        if (prefs.getBoolean(PREF_SENT_TO_APP_INFO, false)) {
            prefs.edit().putBoolean(PREF_SENT_TO_APP_INFO, false).apply()
            if (!InputService.InputBridge.enabled()) {
                remoteHintText = "turn on krapy"
                startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))
            }
        }
        if (::remoteBlock.isInitialized) {
            refreshRemoteBlock()
        }
        if (::statusLine.isInitialized) {
            refreshStatus()
        }
        if (::listView.isInitialized) {
            refreshList()
        }
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus) return
        if (Build.VERSION.SDK_INT < 29) return
        val send = SessionBridge.sendClipboard ?: return
        val cm = getSystemService(CLIPBOARD_SERVICE) as ClipboardManager
        val clip = cm.primaryClip ?: return
        if (clip.itemCount < 1) return
        val text = clip.getItemAt(0).coerceToText(this)?.toString() ?: return
        if (text == MirrorService.lastDesktopClipboard) return
        if (text == MirrorService.lastPhoneClipboardSent) return
        MirrorService.lastPhoneClipboardSent = text
        send(text)
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        try {
            if (requestCode == REQ_SCAN && resultCode == RESULT_OK) {
                val uri = data?.getStringExtra("uri")
                if (uri != null) pairUri(uri) else Toast.makeText(this, "no uri", Toast.LENGTH_SHORT).show()
            }
            if (requestCode == REQ_CAPTURE) {
                val peer = pendingPeer
                pendingPeer = null
                if (peer == null) return
                if (resultCode != RESULT_OK || data == null) {
                    Toast.makeText(this, "capture denied", Toast.LENGTH_SHORT).show()
                    return
                }
                MirrorService.start(
                    this,
                    peer.id,
                    resultCode,
                    data,
                    pendingHostOverride,
                    pendingPortOverride,
                )
                pendingHostOverride = null
                pendingPortOverride = null
                refreshStatus()
            }
        } catch (t: Throwable) {
            CrashLog.record(this, t, "onActivityResult")
            Toast.makeText(this, t.message ?: "capture failed", Toast.LENGTH_LONG).show()
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == REQ_NOTIF) {
            val peer = pendingPeer
            if (peer != null) {
                beginCapture(peer, pendingHostOverride, pendingPortOverride)
            }
        }
    }

    private fun maybeRequestNotif() {
        if (Build.VERSION.SDK_INT < 33) return
        if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) {
            return
        }
        requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), REQ_NOTIF)
    }

    private fun startScan() {
        startActivityForResult(Intent(this, ScanActivity::class.java), REQ_SCAN)
    }

    private fun pairFromField() {
        pairUri(uriField.text.toString())
    }

    private fun handleViewIntent(intent: Intent?) {
        val uri = intent?.data ?: return
        if (intent.action != Intent.ACTION_VIEW) return
        when (uri.host) {
            "pair" -> pairUri(uri.toString())
            "connect" -> connectUri(uri.toString())
        }
    }

    private fun connectUri(raw: String) {
        try {
            val data = Pairing.parseConnect(raw)
            val peer = peers.list().firstOrNull { it.id == data.desktopId }
            if (peer == null) {
                Toast.makeText(
                    this,
                    "this PC isn't paired yet, run krapy connect --usb again",
                    Toast.LENGTH_LONG,
                ).show()
                return
            }
            startMirroring(peer, data.host, data.port)
        } catch (e: PairingException) {
            Toast.makeText(this, e.message, Toast.LENGTH_LONG).show()
        } catch (t: Throwable) {
            CrashLog.record(this, t, "connectUri")
            Toast.makeText(this, t.message ?: "connect failed", Toast.LENGTH_LONG).show()
        }
    }

    private fun pairUri(raw: String) {
        try {
            val data = Pairing.parse(raw)
            val peer = peers.fromPairing(data)
            peers.upsert(peer)
            uriField.setText("")
            refreshList()
            statusLine.text = "connecting to ${data.desktopName}…"
            Toast.makeText(this, "paired ${data.desktopName}", Toast.LENGTH_SHORT).show()
            startMirroring(peer)
        } catch (e: PairingException) {
            Toast.makeText(this, e.message, Toast.LENGTH_LONG).show()
        } catch (t: Throwable) {
            CrashLog.record(this, t, "pairUri")
            Toast.makeText(this, t.message ?: "pair failed", Toast.LENGTH_LONG).show()
        }
    }

    private fun refreshList() {
        peerRows = peers.list()
        emptyPeers.visibility = if (peerRows.isEmpty()) View.VISIBLE else View.GONE
        val adapter = peerAdapter
        if (adapter == null) {
            peerAdapter = PeerAdapter(this, peerRows, textPrimary(), textSecondary()).also { listView.adapter = it }
        } else {
            adapter.update(peerRows)
        }
        refreshStatus()
    }

    private fun refreshStatus() {
        val mirroring = MirrorService.activePeerName
        val base = when {
            mirroring != null -> "mirroring to $mirroring"
            peerRows.isNotEmpty() -> "paired (${peerRows.size} desktop${if (peerRows.size == 1) "" else "s"})"
            else -> "idle"
        }
        val problem = MirrorService.lastProblem
        statusLine.text = if (problem != null) "$base\n$problem" else base
    }

    private fun uiPrefs() = getSharedPreferences(PREFS_UI, Context.MODE_PRIVATE)

    private fun isPlayStoreInstall(): Boolean {
        return try {
            packageManager.getInstallSourceInfo(packageName).installingPackageName == "com.android.vending"
        } catch (_: Exception) {
            false
        }
    }

    private fun refreshRemoteBlock() {
        remoteBlock.removeAllViews()
        if (InputService.InputBridge.enabled()) {
            remoteBlock.visibility = View.GONE
            return
        }
        remoteBlock.visibility = View.VISIBLE
        remoteBlock.addView(TextView(this).apply {
            text = remoteHintText
            textSize = 13f
            setTextColor(textSecondary())
            setPadding(0, 0, 0, dp(8))
        })
        remoteBlock.addView(secondaryButton("enable remote control") {
            onEnableRemoteControlClicked()
        })
        remoteBlock.addView(View(this).apply {
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                dp(16),
            )
        })
    }

    private fun onEnableRemoteControlClicked() {
        if (Build.VERSION.SDK_INT < 33 || isPlayStoreInstall()) {
            startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))
            return
        }
        val message =
            "1. Settings > Apps > krapy (we open it for you).\n" +
            "2. Tap the ⋮ menu (top right) > Allow restricted settings, then confirm with your PIN or fingerprint.\n" +
            "3. Come back here and we open Accessibility: krapy > turn on.\n\n" +
            "Mirroring works without this."
        AlertDialog.Builder(this)
            .setTitle("Allow remote control")
            .setMessage(message)
            .setPositiveButton("Continue") { _, _ ->
                uiPrefs().edit().putBoolean(PREF_SENT_TO_APP_INFO, true).apply()
                startActivity(
                    Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS).apply {
                        data = Uri.fromParts("package", packageName, null)
                    },
                )
            }
            .setNegativeButton("Cancel", null)
            .show()
    }

    private fun startMirroring(
        peer: Peer,
        hostOverride: String? = null,
        portOverride: Int? = null,
    ) {
        try {
            if (Build.VERSION.SDK_INT >= 33 &&
                checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
            ) {
                pendingPeer = peer
                pendingHostOverride = hostOverride
                pendingPortOverride = portOverride
                requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), REQ_NOTIF)
                return
            }
            beginCapture(peer, hostOverride, portOverride)
        } catch (t: Throwable) {
            CrashLog.record(this, t, "startMirroring")
            Toast.makeText(this, t.message ?: "mirror failed", Toast.LENGTH_LONG).show()
        }
    }

    private fun beginCapture(
        peer: Peer,
        hostOverride: String? = null,
        portOverride: Int? = null,
    ) {
        try {
            pendingPeer = peer
            pendingHostOverride = hostOverride
            pendingPortOverride = portOverride
            val mgr = getSystemService(MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
            startActivityForResult(mgr.createScreenCaptureIntent(), REQ_CAPTURE)
        } catch (t: Throwable) {
            CrashLog.record(this, t, "beginCapture")
            Toast.makeText(this, t.message ?: "capture failed", Toast.LENGTH_LONG).show()
        }
    }

    private fun maybeShowLastCrash() {
        val text = CrashLog.readFile(this) ?: return
        val scroll = TextView(this).apply {
            this.text = text
            typeface = Typeface.MONOSPACE
            textSize = 11f
            setTextIsSelectable(true)
            movementMethod = ScrollingMovementMethod()
            setPadding(dp(8), dp(8), dp(8), dp(8))
            maxHeight = dp(280)
        }
        val wrap = android.widget.ScrollView(this).apply {
            addView(scroll)
        }
        AlertDialog.Builder(this)
            .setTitle("krapy crashed last time")
            .setView(wrap)
            .setPositiveButton("Copy") { _, _ ->
                val cm = getSystemService(CLIPBOARD_SERVICE) as ClipboardManager
                cm.setPrimaryClip(ClipData.newPlainText("krapy crash", text))
                Toast.makeText(this, "copied", Toast.LENGTH_SHORT).show()
                CrashLog.deleteFile(this)
            }
            .setNegativeButton("Dismiss") { _, _ ->
                CrashLog.deleteFile(this)
            }
            .setCancelable(false)
            .show()
    }

    private fun confirmForget(peer: Peer) {
        AlertDialog.Builder(this)
            .setMessage("Forget ${peer.name}?")
            .setPositiveButton("Forget") { _, _ ->
                peers.forget(peer.id)
                refreshList()
            }
            .setNegativeButton("Cancel", null)
            .show()
    }

    private fun applyWindowInsets(root: LinearLayout) {
        if (Build.VERSION.SDK_INT >= 30) {
            root.setOnApplyWindowInsetsListener { v, insets ->
                val bars = insets.getInsets(WindowInsets.Type.systemBars())
                val ime = insets.getInsets(WindowInsets.Type.ime())
                val cutout = insets.getInsets(WindowInsets.Type.displayCutout())
                val bottom = max(bars.bottom, ime.bottom)
                v.setPadding(
                    basePadH + max(bars.left, cutout.left),
                    basePadTop + max(bars.top, cutout.top),
                    basePadH + max(bars.right, cutout.right),
                    basePadBottom + bottom,
                )
                insets
            }
        } else {
            @Suppress("DEPRECATION")
            root.setOnApplyWindowInsetsListener { v, insets ->
                val sys = insets.systemWindowInsets
                v.setPadding(
                    basePadH + sys.left,
                    basePadTop + sys.top,
                    basePadH + sys.right,
                    basePadBottom + sys.bottom,
                )
                insets
            }
        }
        root.requestApplyInsets()
    }

    private fun dp(value: Int): Int =
        (value * resources.displayMetrics.density + 0.5f).toInt()

    private fun textPrimary(): Int {
        val tv = TypedValue()
        theme.resolveAttribute(android.R.attr.textColorPrimary, tv, true)
        return tv.data
    }

    private fun textSecondary(): Int {
        val tv = TypedValue()
        theme.resolveAttribute(android.R.attr.textColorSecondary, tv, true)
        return tv.data
    }

    private fun accentColor(): Int = getColor(R.color.accent)

    private fun primaryButton(label: String, onClick: () -> Unit): Button =
        Button(this).apply {
            text = label
            isAllCaps = false
            minHeight = dp(48)
            setTextColor(ColorStateList.valueOf(0xFFFFFFFF.toInt()))
            background = GradientDrawable().apply {
                setColor(accentColor())
                cornerRadius = dp(8).toFloat()
            }
            stateListAnimator = null
            elevation = 0f
            setOnClickListener { onClick() }
        }

    private fun secondaryButton(label: String, onClick: () -> Unit): Button =
        Button(this).apply {
            text = label
            isAllCaps = false
            minHeight = dp(48)
            setTextColor(accentColor())
            background = GradientDrawable().apply {
                setStroke(dp(1), accentColor())
                cornerRadius = dp(8).toFloat()
                setColor(0)
            }
            stateListAnimator = null
            elevation = 0f
            setOnClickListener { onClick() }
        }

    companion object {
        private const val REQ_SCAN = 1
        private const val REQ_CAPTURE = 2
        private const val REQ_NOTIF = 3
        private const val PREFS_UI = "krapy_ui"
        private const val PREF_SENT_TO_APP_INFO = "sent_to_app_info"
    }
}