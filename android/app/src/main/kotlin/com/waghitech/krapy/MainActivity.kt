package com.waghitech.krapy

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.graphics.Color
import android.graphics.Typeface
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
    private var peerRows: List<Peer> = emptyList()

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
        listView.onItemClickListener = AdapterView.OnItemClickListener { _, _, pos, _ ->
            startMirroring(peerRows[pos])
        }
        listView.onItemLongClickListener = AdapterView.OnItemLongClickListener { _, _, pos, _ ->
            confirmForget(peerRows[pos])
            true
        }
        refreshList()
        handlePairIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handlePairIntent(intent)
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == REQ_SCAN && resultCode == RESULT_OK) {
            val uri = data?.getStringExtra("uri")
            if (uri != null) pairUri(uri) else Toast.makeText(this, "no uri", Toast.LENGTH_SHORT).show()
        }
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
            peers.upsert(peers.fromPairing(data))
            uriField.setText("")
            refreshList()
            Toast.makeText(this, "paired ${data.desktopName}", Toast.LENGTH_SHORT).show()
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
        Toast.makeText(this, "mirror ${peer.name} (later)", Toast.LENGTH_SHORT).show()
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
    }
}