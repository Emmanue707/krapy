package com.waghitech.krapy

import java.util.Locale

object NotifFormat {
    fun sessionTitle(peerName: String, reconnect: Boolean): String =
        if (reconnect) "Reconnecting to $peerName" else "Mirroring to $peerName"

    fun contentText(s: SessionState.Snap): String =
        SessionState.statsLine(s)

    fun soundActionLabel(onPc: Boolean): String =
        if (onPc) "Sound: PC" else "Sound: phone"

    fun pauseActionLabel(paused: Boolean): String =
        if (paused) "Resume" else "Pause"
}