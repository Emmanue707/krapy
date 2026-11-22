package com.waghitech.krapy

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import java.net.InetAddress
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

class PeerResolve(private val context: Context) {
    private val nsd = context.getSystemService(Context.NSD_SERVICE) as NsdManager

    fun resolve(desktopId: String, timeoutMs: Long): Pair<String, Int>? {
        val wifi = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        val lock = wifi.createMulticastLock("krapy-mdns")
        lock.setReferenceCounted(false)
        lock.acquire()
        return try {
            resolveLocked(desktopId, timeoutMs)
        } finally {
            if (lock.isHeld) lock.release()
        }
    }

    private fun resolveLocked(desktopId: String, timeoutMs: Long): Pair<String, Int>? {
        val result = AtomicReference<Pair<String, Int>?>(null)
        val latch = CountDownLatch(1)
        val listener = object : NsdManager.DiscoveryListener {
            override fun onStartDiscoveryFailed(type: String?, code: Int) {
                latch.countDown()
            }

            override fun onStopDiscoveryFailed(type: String?, code: Int) {
                latch.countDown()
            }

            override fun onDiscoveryStarted(type: String?) {}

            override fun onDiscoveryStopped(type: String?) {}

            override fun onServiceFound(info: NsdServiceInfo) {
                val id = txtId(info) ?: return
                if (id != desktopId) return
                if (result.get() != null) return
                if (Build.VERSION.SDK_INT >= 34) {
                    nsd.registerServiceInfoCallback(
                        info,
                        Runnable::run,
                        object : NsdManager.ServiceInfoCallback {
                            override fun onServiceUpdated(serviceInfo: NsdServiceInfo) {
                                take(serviceInfo)
                            }

                            override fun onServiceLost() {}

                            override fun onServiceInfoCallbackRegistrationFailed(errorCode: Int) {
                                fallbackResolve(info)
                            }

                            override fun onServiceInfoCallbackUnregistered() {}
                        },
                    )
                } else {
                    fallbackResolve(info)
                }
            }

            override fun onServiceLost(info: NsdServiceInfo) {}

            fun take(info: NsdServiceInfo) {
                val host = pickHost(info.host) ?: return
                val port = info.port
                if (port <= 0) return
                if (result.compareAndSet(null, host to port)) {
                    latch.countDown()
                }
            }

            fun fallbackResolve(info: NsdServiceInfo) {
                nsd.resolveService(
                    info,
                    object : NsdManager.ResolveListener {
                        override fun onResolveFailed(serviceInfo: NsdServiceInfo?, errorCode: Int) {}

                        override fun onServiceResolved(serviceInfo: NsdServiceInfo) {
                            take(serviceInfo)
                        }
                    },
                )
            }
        }
        try {
            nsd.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, listener)
        } catch (_: Exception) {
            return null
        }
        latch.await(timeoutMs, TimeUnit.MILLISECONDS)
        try {
            nsd.stopServiceDiscovery(listener)
        } catch (_: Exception) {
        }
        return result.get()
    }

    private fun txtId(info: NsdServiceInfo): String? {
        val v = info.attributes["id"] ?: return null
        return String(v, Charsets.UTF_8)
    }

    private fun pickHost(host: InetAddress?): String? {
        if (host == null) return null
        val s = host.hostAddress ?: return null
        if (s.contains('%')) return s.substringBefore('%')
        return s
    }

    companion object {
        private const val SERVICE_TYPE = "_krapy._tcp."

        fun probeTcp(host: String, port: Int): Boolean {
            return try {
                java.net.Socket().use { s ->
                    s.connect(java.net.InetSocketAddress(host, port), 2000)
                    true
                }
            } catch (_: Exception) {
                false
            }
        }

        fun raceEndpoint(
            context: Context,
            storedHost: String,
            storedPort: Int,
            desktopId: String,
        ): Pair<String, Int>? {
            val found = AtomicReference<Pair<String, Int>?>(null)
            val done = CountDownLatch(1)
            fun win(h: String, p: Int) {
                if (found.compareAndSet(null, h to p)) done.countDown()
            }
            val t1 = kotlin.concurrent.thread {
                if (probeTcp(storedHost, storedPort)) win(storedHost, storedPort)
            }
            val t2 = kotlin.concurrent.thread {
                val r = PeerResolve(context).resolve(desktopId, 5000)
                if (r != null && probeTcp(r.first, r.second)) win(r.first, r.second)
            }
            done.await(6, TimeUnit.SECONDS)
            t1.interrupt()
            t2.interrupt()
            return found.get()
        }
    }
}