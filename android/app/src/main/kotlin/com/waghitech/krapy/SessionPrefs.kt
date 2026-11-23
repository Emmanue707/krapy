package com.waghitech.krapy

import android.content.Context

object SessionPrefs {
    private const val PREFS = "krapy_session"

    const val PREF_PRESET = "preset"
    const val PREF_SOUND_PC = "sound_pc"
    const val PREF_KEEP_AWAKE = "keep_awake"
    const val PREF_CLIPBOARD = "clipboard_sync"

    const val PRESET_SMOOTH = 0
    const val PRESET_BALANCED = 1
    const val PRESET_SHARP = 2

    fun prefs(ctx: Context) = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    fun preset(ctx: Context): Int =
        prefs(ctx).getInt(PREF_PRESET, PRESET_BALANCED).coerceIn(PRESET_SMOOTH, PRESET_SHARP)

    fun soundOnPc(ctx: Context): Boolean = prefs(ctx).getBoolean(PREF_SOUND_PC, false)

    fun keepAwake(ctx: Context): Boolean = prefs(ctx).getBoolean(PREF_KEEP_AWAKE, true)

    fun clipboardSync(ctx: Context): Boolean = prefs(ctx).getBoolean(PREF_CLIPBOARD, true)

    fun presetLabel(id: Int): String = when (id.coerceIn(PRESET_SMOOTH, PRESET_SHARP)) {
        PRESET_SMOOTH -> "Smooth"
        PRESET_BALANCED -> "Balanced"
        else -> "Sharp"
    }
}