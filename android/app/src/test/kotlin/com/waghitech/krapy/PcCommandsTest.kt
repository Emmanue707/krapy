package com.waghitech.krapy

import org.junit.Assert.assertEquals
import org.junit.Test

class PcCommandsTest {
    @Test
    fun mapsSettingKeysToServiceActions() {
        assertEquals(MirrorService.ACTION_PAUSE_FROM_PC, PcCommands.serviceAction(PcCommands.KEY_PAUSE))
        assertEquals(MirrorService.ACTION_PRESET_FROM_PC, PcCommands.serviceAction(PcCommands.KEY_PRESET))
        assertEquals(MirrorService.ACTION_SOUND_PC, PcCommands.serviceAction(PcCommands.KEY_SOUND_PC))
        assertEquals(MirrorService.ACTION_WAKE, PcCommands.serviceAction(PcCommands.KEY_KEEP_AWAKE))
        assertEquals(MirrorService.ACTION_CLIPBOARD, PcCommands.serviceAction(PcCommands.KEY_CLIPBOARD))
        assertEquals(MirrorService.ACTION_ROTATE, PcCommands.serviceAction(PcCommands.KEY_ROTATE))
        assertEquals(MirrorService.ACTION_DISCONNECT, PcCommands.serviceAction(PcCommands.KEY_DISCONNECT))
    }

    @Test
    fun toggleSentinel() {
        assertEquals(0, PcCommands.normalizedValue(PcCommands.KEY_PAUSE, 0xff, true, false, true, true))
        assertEquals(1, PcCommands.normalizedValue(PcCommands.KEY_PAUSE, 0xff, false, false, true, true))
    }
}