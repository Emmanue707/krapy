package com.waghitech.krapy

import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import kotlin.concurrent.thread

object Workers {
    @Volatile
    var onError: ((String, Throwable) -> Unit)? = null

    fun guard(name: String, body: () -> Unit) {
        try {
            body()
        } catch (_: InterruptedException) {
        } catch (t: Throwable) {
            try {
                onError?.invoke(name, t)
            } catch (_: Throwable) {
            }
        }
    }

    fun start(name: String, body: () -> Unit): Thread =
        thread(name = name, isDaemon = true) { guard(name, body) }

    // first non-null answer wins; the losers are interrupted
    fun <T> race(timeoutMs: Long, vararg tasks: () -> T?): T? {
        val found = AtomicReference<T?>(null)
        val done = CountDownLatch(1)
        val left = java.util.concurrent.atomic.AtomicInteger(tasks.size)
        val threads = tasks.mapIndexed { i, task ->
            start("krapy-race-$i") {
                try {
                    val r = task()
                    if (r != null && found.compareAndSet(null, r)) done.countDown()
                } finally {
                    if (left.decrementAndGet() == 0) done.countDown()
                }
            }
        }
        try {
            done.await(timeoutMs, TimeUnit.MILLISECONDS)
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
        } finally {
            threads.forEach { it.interrupt() }
        }
        return found.get()
    }
}
