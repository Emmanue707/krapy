package com.waghitech.krapy

object PcCommands {
    const val KEY_PAUSE = 1
    const val KEY_PRESET = 2
    const val KEY_SOUND_PC = 3
    const val KEY_KEEP_AWAKE = 4
    const val KEY_CLIPBOARD = 5
    const val KEY_ROTATE = 6
    const val KEY_DISCONNECT = 7

    fun serviceAction(key: Int): String? = when (key) {
        KEY_PAUSE -> MirrorService.ACTION_PAUSE_FROM_PC
        KEY_PRESET -> MirrorService.ACTION_PRESET_FROM_PC
        KEY_SOUND_PC -> MirrorService.ACTION_SOUND_PC
        KEY_KEEP_AWAKE -> MirrorService.ACTION_WAKE
        KEY_CLIPBOARD -> MirrorService.ACTION_CLIPBOARD
        KEY_ROTATE -> MirrorService.ACTION_ROTATE
        KEY_DISCONNECT -> MirrorService.ACTION_DISCONNECT
        else -> null
    }

    fun normalizedValue(key: Int, value: Int, paused: Boolean, soundPc: Boolean, keepAwake: Boolean, clipboard: Boolean): Int =
        when (key) {
            KEY_PAUSE -> if (value == 0xff) if (paused) 0 else 1 else value
            KEY_SOUND_PC -> if (value == 0xff) if (soundPc) 0 else 1 else value
            KEY_KEEP_AWAKE -> if (value == 0xff) if (keepAwake) 0 else 1 else value
            KEY_CLIPBOARD -> if (value == 0xff) if (clipboard) 0 else 1 else value
            else -> value
        }
}