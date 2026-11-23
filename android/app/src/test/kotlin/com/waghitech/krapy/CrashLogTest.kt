package com.waghitech.krapy

import org.junit.Assert.assertTrue
import org.junit.Test

class CrashLogTest {
    @Test
    fun formatIncludesVersionModelSdkAndNestedCause() {
        val device = CrashLogDevice(
            versionName = "0.1.3",
            versionCode = 4,
            manufacturer = "TestMaker",
            model = "TestPhone",
            sdkInt = 34,
        )
        val root = IllegalStateException("outer fail")
        val nested = RuntimeException("inner reason")
        root.initCause(nested)
        val out = CrashLog.format(Thread.currentThread(), root, device, "2026-01-01T00:00:00Z")
        assertTrue(out.contains("0.1.3"))
        assertTrue(out.contains("TestMaker"))
        assertTrue(out.contains("TestPhone"))
        assertTrue(out.contains("api 34"))
        assertTrue(out.contains("IllegalStateException"))
        assertTrue(out.contains("outer fail"))
        assertTrue(out.contains("Caused by"))
        assertTrue(out.contains("RuntimeException"))
        assertTrue(out.contains("inner reason"))
    }
}