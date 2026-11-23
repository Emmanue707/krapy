package com.waghitech.krapy

object Annex {
    fun hasSps(data: ByteArray): Boolean {
        var i = 0
        val n = data.size
        while (i + 3 < n) {
            if (data[i].toInt() == 0 && data[i + 1].toInt() == 0) {
                if (data[i + 2].toInt() == 1) {
                    if (data[i + 3].toInt() and 0x1f == 7) return true
                    i += 3
                    continue
                }
                if (data[i + 2].toInt() == 0 && i + 4 < n && data[i + 3].toInt() == 1) {
                    if (data[i + 4].toInt() and 0x1f == 7) return true
                    i += 4
                    continue
                }
            }
            i++
        }
        return false
    }
}
