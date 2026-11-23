package com.waghitech.krapy

import java.util.Locale

object SessionState {
    data class Snap(
        val peerName: String = "",
        val transport: String = "",
        val fps: Int = 0,
        val latencyMs: Int = 0,
        val bitrateBps: Int = 0,
        val paused: Boolean = false,
    )

    @Volatile
    var snap: Snap = Snap()

    @Volatile
    var mirroring: Boolean = false

    @Volatile
    var audioOnPc: Boolean = false

    @Volatile
    var clipboardSync: Boolean = true

    fun statsLine(s: Snap = snap): String {
        val mbps = s.bitrateBps / 1_000_000.0
        return "${s.fps} fps · ${s.latencyMs} ms · ${String.format(Locale.US, "%.1f", mbps)} Mbps"
    }
}