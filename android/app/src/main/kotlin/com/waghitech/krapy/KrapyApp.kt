package com.waghitech.krapy

import android.app.Application

class KrapyApp : Application() {
    override fun onCreate() {
        super.onCreate()
        val prev = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { thread, t ->
            CrashLog.writeUncaught(applicationContext, thread, t)
            prev?.uncaughtException(thread, t)
        }
    }
}