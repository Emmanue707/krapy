package com.waghitech.krapy

import android.content.Context

object SessionPrefs {
    private const val PREFS = "krapy_session"

    const val PREF_PRESET = "preset"
    const val PREF_SOUND_PC = "sound_pc"
    const val PREF_KEEP_AWAKE = "keep_awake"
    const val PREF_CLIPBOARD = "clipboard_sync"
    const val PREF_SAVED_MUSIC_VOL = "saved_music_vol"

    const val PRESET_SMOOTH = 0
    const val PRESET_BALANCED = 1
    const val PRESET_SHARP = 2

    fun prefs(ctx: Context) = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    fun preset(ctx: Context): Int =
        prefs(ctx).getInt(PREF_PRESET, PRESET_BALANCED).coerceIn(PRESET_SMOOTH, PRESET_SHARP)

    fun soundOnPc(ctx: Context): Boolean = prefs(ctx).getBoolean(PREF_SOUND_PC, false)

    fun keepAwake(ctx: Context): Boolean = prefs(ctx).getBoolean(PREF_KEEP_AWAKE, true)

    fun clipboardSync(ctx: Context): Boolean = prefs(ctx).getBoolean(PREF_CLIPBOARD, true)

    fun setSavedMusicVolume(ctx: Context, vol: Int) {
        prefs(ctx).edit().putInt(PREF_SAVED_MUSIC_VOL, vol).apply()
    }

    fun takeSavedMusicVolume(ctx: Context): Int? {
        val v = prefs(ctx).getInt(PREF_SAVED_MUSIC_VOL, -1)
        if (v < 0) return null
        prefs(ctx).edit().remove(PREF_SAVED_MUSIC_VOL).apply()
        return v
    }

    fun presetLabel(id: Int): String = when (id.coerceIn(PRESET_SMOOTH, PRESET_SHARP)) {
        PRESET_SMOOTH -> "Smooth"
        PRESET_BALANCED -> "Balanced"
        else -> "Sharp"
    }
}