package com.waghitech.krapy

object SessionBridge {
    @Volatile
    var sendClipboard: ((String) -> Unit)? = null

    @Volatile
    var clipboardSync: Boolean = true
}