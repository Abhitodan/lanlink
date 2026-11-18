package com.lanlink.app.core

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.ext.SdkExtensions
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import java.net.Inet4Address
import java.util.concurrent.Executors

/** A PC seen on the network through mDNS. Nothing here is trusted. */
data class FoundHost(
    val serviceName: String,
    val name: String,
    val shortId: String,
    val address: String,
    val port: Int,
    val protocol: Int,
)

/**
 * Browses `_lanlink._udp` with the platform's `NsdManager` while the host
 * list is visible, and stops when it is not (no background scanning).
 *
 * Targeting API 35, NsdManager works without extra permissions. Android 17
 * (API 37) apps must either hold `ACCESS_LOCAL_NETWORK` or use the system
 * picker; see docs/adr/0011.
 */
class Discovery(context: Context) {
    private val nsd = context.getSystemService(Context.NSD_SERVICE) as NsdManager
    private val wifi = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
    private val executor = Executors.newSingleThreadExecutor()
    private val _hosts = MutableStateFlow<List<FoundHost>>(emptyList())
    val hosts: StateFlow<List<FoundHost>> = _hosts
    private val _status = MutableStateFlow("idle")
    val status: StateFlow<String> = _status

    /** `registerServiceInfoCallback` needs Android 14 with T-extension 7. */
    private val infoCallbacks: Boolean =
        Build.VERSION.SDK_INT >= 34 && SdkExtensions.getExtensionVersion(Build.VERSION_CODES.TIRAMISU) >= 7

    private var listener: NsdManager.DiscoveryListener? = null
    private var lock: WifiManager.MulticastLock? = null
    private val callbacks = mutableMapOf<String, NsdManager.ServiceInfoCallback>()
    private val pendingResolves = ArrayDeque<NsdServiceInfo>()
    private var resolvingLegacy = false

    @Synchronized
    fun start() {
        if (listener != null) return
        _hosts.value = emptyList()
        lock = wifi.createMulticastLock("lanlink-discovery").apply {
            setReferenceCounted(false)
            runCatching { acquire() }
        }
        val l = object : NsdManager.DiscoveryListener {
            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                _status.value = "failed:$errorCode"
                // Forget the failed listener so the next start() retries.
                synchronized(this@Discovery) { if (listener === this) listener = null }
            }
            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {}
            override fun onDiscoveryStarted(serviceType: String) { _status.value = "searching" }
            override fun onDiscoveryStopped(serviceType: String) { _status.value = "idle" }
            override fun onServiceFound(info: NsdServiceInfo) = resolve(info)
            override fun onServiceLost(info: NsdServiceInfo) {
                _hosts.value = _hosts.value.filter { it.serviceName != info.serviceName }
                synchronized(this@Discovery) {
                    val cb = callbacks.remove(info.serviceName)
                    if (cb != null && infoCallbacks) unregister(cb)
                }
            }
        }
        listener = l
        runCatching { nsd.discoverServices("_lanlink._udp", NsdManager.PROTOCOL_DNS_SD, l) }
            .onFailure { _status.value = "failed"; listener = null }
    }

    @Synchronized
    fun stop() {
        listener?.let { runCatching { nsd.stopServiceDiscovery(it) } }
        listener = null
        if (infoCallbacks) callbacks.values.forEach { unregister(it) }
        callbacks.clear()
        pendingResolves.clear()
        resolvingLegacy = false
        runCatching { lock?.release() }
        lock = null
    }

    @android.annotation.SuppressLint("NewApi") // guarded by `infoCallbacks`
    private fun unregister(cb: NsdManager.ServiceInfoCallback) {
        runCatching { nsd.unregisterServiceInfoCallback(cb) }
    }

    private fun publish(info: NsdServiceInfo, rawAddress: String) {
        val address = rawAddress.substringBefore('%').trim()
        if (address.isEmpty()) return
        fun txt(k: String) = info.attributes[k]?.let { String(it, Charsets.UTF_8) }
        val h = FoundHost(
            serviceName = info.serviceName,
            // Any device can advertise any name; clean it before showing it.
            name = DisplayText.name(txt("n") ?: info.serviceName).ifEmpty { "PC" },
            shortId = txt("id") ?: "",
            address = address,
            port = info.port,
            protocol = txt("v")?.toIntOrNull() ?: 0,
        )
        _hosts.value = (_hosts.value.filter { it.serviceName != h.serviceName } + h).sortedBy { it.name }
    }

    private fun resolve(info: NsdServiceInfo) {
        if (infoCallbacks) {
            resolveWithCallback(info)
        } else {
            enqueueLegacyResolve(info)
        }
    }

    @Synchronized
    private fun enqueueLegacyResolve(info: NsdServiceInfo) {
        if (pendingResolves.none { it.serviceName == info.serviceName }) {
            pendingResolves.addLast(info)
        }
        if (!resolvingLegacy) drainLegacyResolves()
    }

    @Synchronized
    private fun drainLegacyResolves() {
        val next = pendingResolves.removeFirstOrNull()
        if (next == null) {
            resolvingLegacy = false
            return
        }
        resolvingLegacy = true
        @Suppress("DEPRECATION")
        runCatching {
            nsd.resolveService(next, object : NsdManager.ResolveListener {
                override fun onResolveFailed(si: NsdServiceInfo, errorCode: Int) {
                    drainLegacyResolves()
                }
                override fun onServiceResolved(si: NsdServiceInfo) {
                    @Suppress("DEPRECATION")
                    si.host?.hostAddress?.let { publish(si, it) }
                    drainLegacyResolves()
                }
            })
        }.onFailure {
            drainLegacyResolves()
        }
    }

    @android.annotation.SuppressLint("NewApi") // only called when `infoCallbacks`
    private fun resolveWithCallback(info: NsdServiceInfo) {
        run {
            val cb = object : NsdManager.ServiceInfoCallback {
                override fun onServiceInfoCallbackRegistrationFailed(errorCode: Int) {
                    // Allow a later onServiceFound to try again.
                    synchronized(this@Discovery) {
                        if (callbacks[info.serviceName] === this) callbacks.remove(info.serviceName)
                    }
                }
                override fun onServiceUpdated(si: NsdServiceInfo) {
                    val v4 = si.hostAddresses.firstOrNull { it is Inet4Address } ?: si.hostAddresses.firstOrNull()
                    v4?.hostAddress?.let { publish(si, it) }
                }
                override fun onServiceLost() {
                    _hosts.value = _hosts.value.filter { it.serviceName != info.serviceName }
                }
                override fun onServiceInfoCallbackUnregistered() {}
            }
            synchronized(this) {
                if (callbacks.containsKey(info.serviceName)) return
                callbacks[info.serviceName] = cb
            }
            runCatching { nsd.registerServiceInfoCallback(info, executor, cb) }
                .onFailure { synchronized(this) { if (callbacks[info.serviceName] === cb) callbacks.remove(info.serviceName) } }
        }
    }
}
