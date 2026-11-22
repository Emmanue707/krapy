package com.waghitech.krapy

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.content.ClipboardManager
import android.content.Intent
import android.content.pm.PackageManager
import android.provider.Settings
import android.graphics.Color
import android.graphics.Typeface
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ListView
import android.widget.TextView
import android.widget.Toast
class MainActivity : Activity() {
    private lateinit var peers: Peers
    private lateinit var listView: ListView
    private lateinit var uriField: EditText
    private lateinit var inputHint: TextView
    private lateinit var inputBtn: Button
    private var peerRows: List<Peer> = emptyList()
    private var pendingPeer: Peer? = null
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        peers = Peers(this)
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(32, 48, 32, 32)
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
        }
        val title = TextView(this).apply {
            text = "krapy"
            textSize = 28f
            setTypeface(typeface, Typeface.BOLD)
            setTextColor(Color.BLACK)
        }
        root.addView(title)
        val phoneLine = TextView(this).apply {
            text = "${peers.phoneName()} · ${peers.phoneId()}"
            textSize = 13f
            setTextColor(Color.DKGRAY)
            setPadding(0, 8, 0, 24)
        }
        root.addView(phoneLine)
        inputHint = TextView(this).apply {
            textSize = 13f
            setTextColor(Color.DKGRAY)
            setPadding(0, 0, 0, 12)
            visibility = View.GONE
        }
        root.addView(inputHint)
        inputBtn = Button(this).apply {
            text = "enable remote control"
            visibility = View.GONE
            setOnClickListener {
                startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))
            }
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
            ).apply { bottomMargin = 12 }
        }
        root.addView(inputBtn)
        listView = ListView(this).apply {
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                0,
                1f,
            )
        }
        root.addView(listView)
        val scan = Button(this).apply {
            text = "Scan QR"
            setOnClickListener { startScan() }
        }
        root.addView(scan)
        uriField = EditText(this).apply {
            hint = "krapy://pair?..."
            setSingleLine()
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
            ).apply { topMargin = 16 }
        }
        root.addView(uriField)
        val pair = Button(this).apply {
            text = "Pair"
            setOnClickListener { pairFromField() }
        }
        root.addView(pair)
        setContentView(root)
        refreshInputHint(inputHint, inputBtn)
        listView.onItemClickListener = AdapterView.OnItemClickListener { _, _, pos, _ ->
            startMirroring(peerRows[pos])
        }
        listView.onItemLongClickListener = AdapterView.OnItemLongClickListener { _, _, pos, _ ->
            confirmForget(peerRows[pos])
            true
        }
        refreshList()
        handlePairIntent(intent)
        maybeRequestNotif()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handlePairIntent(intent)
    }

    override fun onResume() {
        super.onResume()
        if (::inputHint.isInitialized && ::inputBtn.isInitialized) {
            refreshInputHint(inputHint, inputBtn)
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
            MirrorService.start(this, peer.id, resultCode, data)
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == REQ_NOTIF) {
            val peer = pendingPeer
            if (peer != null) beginCapture(peer)
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

    private fun handlePairIntent(intent: Intent?) {
        val uri = intent?.data?.toString() ?: return
        if (intent.action != Intent.ACTION_VIEW) return
        pairUri(uri)
    }

    private fun pairUri(raw: String) {
        try {
            val data = Pairing.parse(raw)
            val peer = peers.fromPairing(data)
            peers.upsert(peer)
            uriField.setText("")
            refreshList()
            Toast.makeText(this, "paired ${data.desktopName}", Toast.LENGTH_SHORT).show()
            startMirroring(peer)
        } catch (e: PairingException) {
            Toast.makeText(this, e.message, Toast.LENGTH_LONG).show()
        }
    }

    private fun refreshList() {
        peerRows = peers.list()
        val labels = peerRows.map { "${it.name}\n${it.host}:${it.port}" }
        listView.adapter = object : ArrayAdapter<String>(this, android.R.layout.simple_list_item_2, labels) {
            override fun getView(position: Int, convertView: View?, parent: ViewGroup): View {
                val v = super.getView(position, convertView, parent) as TextView
                v.setTextColor(Color.BLACK)
                v.gravity = Gravity.CENTER_VERTICAL
                return v
            }
        }
    }

    private fun startMirroring(peer: Peer) {
        if (Build.VERSION.SDK_INT >= 33 &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            pendingPeer = peer
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), REQ_NOTIF)
            return
        }
        beginCapture(peer)
    }

    private fun beginCapture(peer: Peer) {
        pendingPeer = peer
        val mgr = getSystemService(MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        startActivityForResult(mgr.createScreenCaptureIntent(), REQ_CAPTURE)
    }

    private fun refreshInputHint(hint: TextView, btn: Button) {
        if (InputService.InputBridge.enabled()) {
            hint.visibility = View.GONE
            btn.visibility = View.GONE
            return
        }
        hint.visibility = View.VISIBLE
        btn.visibility = View.VISIBLE
        hint.text = "remote control needs accessibility"
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

    companion object {
        private const val REQ_SCAN = 1
        private const val REQ_CAPTURE = 2
        private const val REQ_NOTIF = 3
    }
}