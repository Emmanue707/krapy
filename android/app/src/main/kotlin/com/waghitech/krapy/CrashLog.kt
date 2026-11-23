package com.waghitech.krapy

import android.content.Context
import android.os.Build
import java.io.File
import java.time.Instant

data class CrashLogDevice(
    val versionName: String,
    val versionCode: Long,
    val manufacturer: String,
    val model: String,
    val sdkInt: Int,
)

object CrashLog {
    private const val FILE = "last_crash.txt"

    fun format(
        thread: Thread,
        t: Throwable,
        device: CrashLogDevice,
        timeIso: String = Instant.now().toString(),
    ): String {
        val sb = StringBuilder()
        sb.append("version ").append(device.versionName).append(" (").append(device.versionCode).append(")\n")
        sb.append(device.manufacturer).append(" ").append(device.model).append(" api ").append(device.sdkInt).append("\n")
        sb.append(timeIso).append("\n")
        sb.append("thread ").append(thread.name).append("\n\n")
        appendThrowable(sb, t, "")
        return sb.toString()
    }

    fun deviceFrom(context: Context): CrashLogDevice {
        val pi = context.packageManager.getPackageInfo(context.packageName, 0)
        val vn = pi.versionName ?: "?"
        val vc = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            pi.longVersionCode
        } else {
            @Suppress("DEPRECATION")
            pi.versionCode.toLong()
        }
        return CrashLogDevice(
            versionName = vn,
            versionCode = vc,
            manufacturer = Build.MANUFACTURER,
            model = Build.MODEL,
            sdkInt = Build.VERSION.SDK_INT,
        )
    }

    fun record(context: Context, t: Throwable, where: String) {
        val dev = deviceFrom(context)
        val body = "non-fatal in $where\n\n" + format(Thread.currentThread(), t, dev)
        writeFile(context, body)
    }

    fun writeUncaught(context: Context, thread: Thread, t: Throwable) {
        writeFile(context, format(thread, t, deviceFrom(context)))
    }

    fun readFile(context: Context): String? {
        val f = File(context.filesDir, FILE)
        if (!f.isFile) return null
        return try {
            f.readText()
        } catch (_: Exception) {
            null
        }
    }

    fun deleteFile(context: Context) {
        try {
            File(context.filesDir, FILE).delete()
        } catch (_: Exception) {
        }
    }

    private fun writeFile(context: Context, text: String) {
        try {
            File(context.filesDir, FILE).writeText(text)
        } catch (_: Exception) {
        }
    }

    private fun appendThrowable(sb: StringBuilder, t: Throwable, prefix: String) {
        if (prefix.isNotEmpty()) {
            sb.append("Caused by: ")
        }
        sb.append(t.javaClass.name)
        val msg = t.message
        if (!msg.isNullOrEmpty()) {
            sb.append(": ").append(msg)
        }
        sb.append('\n')
        for (el in t.stackTrace) {
            sb.append("  at ").append(el).append('\n')
        }
        val cause = t.cause
        if (cause != null && cause !== t) {
            appendThrowable(sb, cause, "cause")
        }
    }
}