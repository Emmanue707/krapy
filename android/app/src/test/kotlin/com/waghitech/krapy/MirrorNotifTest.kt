package com.waghitech.krapy

import android.app.Notification
import android.app.NotificationManager
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class MirrorNotifTest {
    private val ctx = RuntimeEnvironment.getApplication()

    @Test
    fun channelImportanceDefault() {
        MirrorService.ensureSessionChannel(ctx)
        val nm = ctx.getSystemService(android.content.Context.NOTIFICATION_SERVICE) as NotificationManager
        val ch = nm.getNotificationChannel(MirrorService.SESSION_CHANNEL_ID)
        assertNotNull(ch)
        assertEquals(NotificationManager.IMPORTANCE_DEFAULT, ch!!.importance)
    }

    @Test
    fun threeActionsWithServiceIntentsAndCustomView() {
        val snap = SessionState.Snap(
            peerName = "wcp",
            transport = "Wi‑Fi",
            fps = 60,
            latencyMs = 12,
            bitrateBps = 8_000_000,
        )
        val notif = MirrorService.buildSessionNotification(
            ctx,
            peerName = snap.peerName,
            stats = NotifFormat.contentText(snap),
            paused = false,
            audioOnPc = true,
            reconnect = false,
        )
        assertTrue(
            notif.extras.getString(Notification.EXTRA_TEMPLATE)
                ?.endsWith("DecoratedCustomViewStyle") == true,
        )
        assertEquals(3, notif.actions.size)
        val actions = notif.actions
        assertEquals(MirrorService.ACTION_TOGGLE_PAUSE, intentAction(actions[0].actionIntent))
        assertEquals(MirrorService.ACTION_TOGGLE_AUDIO, intentAction(actions[1].actionIntent))
        assertEquals(MirrorService.ACTION_DISCONNECT, intentAction(actions[2].actionIntent))
        assertEquals(NotifFormat.pauseActionLabel(false), notif.actions[0].title)
    }

    @Test
    fun pauseLabelFlips() {
        val running = MirrorService.buildSessionNotification(
            ctx,
            peerName = "pc",
            stats = "60 fps · 10 ms · 8.0 Mbps",
            paused = false,
            audioOnPc = false,
            reconnect = false,
        )
        assertEquals("Pause", running.actions[0].title)

        val frozen = MirrorService.buildSessionNotification(
            ctx,
            peerName = "pc",
            stats = "0 fps · 10 ms · 8.0 Mbps",
            paused = true,
            audioOnPc = false,
            reconnect = false,
        )
        assertEquals("Resume", frozen.actions[0].title)
    }

    private fun intentAction(pi: android.app.PendingIntent): String? {
        val intent = org.robolectric.Shadows.shadowOf(pi).savedIntent
        return intent.action
    }
}