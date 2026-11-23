package com.waghitech.krapy

import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

class WorkersTest {
    private val uncaught = Collections.synchronizedList(mutableListOf<Throwable>())
    private val reported = Collections.synchronizedList(mutableListOf<String>())
    private var prev: Thread.UncaughtExceptionHandler? = null

    @Before
    fun setUp() {
        prev = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { _, t -> uncaught.add(t) }
        Workers.onError = { where, t -> reported.add("$where: ${t.message}") }
    }

    @After
    fun tearDown() {
        Thread.setDefaultUncaughtExceptionHandler(prev)
        Workers.onError = null
    }

    @Test
    fun interruptedWaitEndsQuietly() {
        val waiting = CountDownLatch(1)
        val t = Workers.start("w") {
            waiting.countDown()
            CountDownLatch(1).await(10, TimeUnit.SECONDS)
        }
        waiting.await()
        t.interrupt()
        t.join(2000)
        assertTrue(!t.isAlive)
        assertTrue(uncaught.isEmpty())
        assertTrue(reported.isEmpty())
    }

    @Test
    fun errorsAreReportedNotThrown() {
        Workers.start("enc") { error("boom") }.join(2000)
        assertTrue(uncaught.isEmpty())
        assertEquals(listOf("enc: boom"), reported)
    }

    @Test
    fun raceTakesFirstAnswerAndStopsTheSlowOne() {
        val r = Workers.race(
            3000,
            { "fast" },
            {
                CountDownLatch(1).await(10, TimeUnit.SECONDS)
                "slow"
            },
        )
        assertEquals("fast", r)
        Thread.sleep(200)
        assertTrue(uncaught.isEmpty())
    }

    @Test
    fun raceGivesUpEarlyWhenEveryoneFails() {
        val t0 = System.nanoTime()
        val r = Workers.race<String>(5000, { null }, { null })
        assertNull(r)
        assertTrue(System.nanoTime() - t0 < 2_000_000_000L)
    }

    @Test
    fun interruptedRaceReturnsNull() {
        var r: String? = "unset"
        val t = Workers.start("reconn") {
            r = Workers.race(5000, { CountDownLatch(1).await(10, TimeUnit.SECONDS); "late" })
        }
        Thread.sleep(100)
        t.interrupt()
        t.join(2000)
        assertNull(r)
        Thread.sleep(200)
        assertTrue(uncaught.isEmpty())
    }
}
