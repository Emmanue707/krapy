package com.waghitech.krapy

import android.widget.LinearLayout
import android.widget.ListView
import android.widget.TextView
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class PeerAdapterTest {
    private fun peer(name: String, host: String) =
        Peer("f72cd30ccd34ad93", name, ByteArray(32), host, 27183, 0L)

    @Test
    fun rowShowsNameAndEndpoint() {
        val ctx = RuntimeEnvironment.getApplication()
        val list = ListView(ctx)
        val adapter = PeerAdapter(ctx, listOf(peer("wcp", "192.168.100.234")), 0, 0)
        list.adapter = adapter
        val row = adapter.getView(0, null, list) as LinearLayout
        assertEquals("wcp", (row.getChildAt(0) as TextView).text.toString())
        assertEquals("192.168.100.234:27183", (row.getChildAt(1) as TextView).text.toString())
    }

    @Test
    fun listLaysOutWithRows() {
        val ctx = RuntimeEnvironment.getApplication()
        val list = ListView(ctx)
        list.adapter = PeerAdapter(ctx, listOf(peer("a", "10.0.0.2"), peer("b", "10.0.0.3")), 0, 0)
        list.measure(
            android.view.View.MeasureSpec.makeMeasureSpec(1080, android.view.View.MeasureSpec.EXACTLY),
            android.view.View.MeasureSpec.makeMeasureSpec(1920, android.view.View.MeasureSpec.EXACTLY),
        )
        list.layout(0, 0, 1080, 1920)
        assertEquals(2, list.childCount)
    }
}
