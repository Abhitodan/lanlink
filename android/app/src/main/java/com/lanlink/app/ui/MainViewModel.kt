package com.lanlink.app.ui

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.lanlink.app.LanLinkApp
import com.lanlink.app.core.NativeBridge
import com.lanlink.app.core.PairedHost
import com.lanlink.app.session.RemoteSession
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

sealed interface Screen {
    data object Onboarding : Screen
    data object Home : Screen
    data class Pair(val prefillAddress: String = "", val prefillPort: Int = 47990) : Screen
    data class Host(val fingerprint: String) : Screen
    data object Remote : Screen
    data object Settings : Screen
    data object Diagnostics : Screen
    data object Security : Screen
}

/** Pairing progress shown on the pairing screen. */
sealed interface PairState {
    data object Idle : PairState
    data class Working(val stage: String) : PairState
    /** A pairing link from outside the app, waiting for the user's yes. */
    data class Offer(val name: String, val addresses: List<String>, val port: Int, val fingerprint: String, val code: String) : PairState
    /** The phone proved the code; waiting for the PC user. */
    data class Confirm(val code: String) : PairState
    data class Done(val host: PairedHost) : PairState
    data class Failed(val message: String) : PairState
}

class MainViewModel(application: Application) : AndroidViewModel(application) {
    val app = application as LanLinkApp

    private val stack = ArrayDeque<Screen>()
    private val _screen = MutableStateFlow<Screen>(if (app.settings.onboarded) Screen.Home else Screen.Onboarding)
    val screen: StateFlow<Screen> = _screen

    private val _pair = MutableStateFlow<PairState>(PairState.Idle)
    val pair: StateFlow<PairState> = _pair
    private var pairJob: Job? = null
    private var pairHandle = 0L

    private val _hosts = MutableStateFlow(app.hosts.all())
    val hosts: StateFlow<List<PairedHost>> = _hosts

    var session: RemoteSession? = null
        private set
    private val _sessionVersion = MutableStateFlow(0)
    val sessionVersion: StateFlow<Int> = _sessionVersion

    fun refreshHosts() { _hosts.value = app.hosts.all() }

    fun go(s: Screen) {
        stack.addLast(_screen.value)
        _screen.value = s
    }

    /** Replace the current screen (no back entry). */
    fun replace(s: Screen) { _screen.value = s }

    fun back(): Boolean {
        if (_screen.value == Screen.Remote) {
            endSession()
        }
        val prev = stack.removeLastOrNull() ?: return false
        _screen.value = prev
        return true
    }

    fun finishOnboarding(next: Screen) {
        app.settings.onboarded = true
        stack.clear()
        _screen.value = Screen.Home
        if (next != Screen.Home) go(next)
    }

    // ------------------------------------------------------------ pairing

    /** Handle a QR code the user just scanned in the app. */
    fun pairFromUri(uri: String) {
        val o = parseOffer(uri) ?: return
        startPairing(o.addresses, o.port, o.fingerprint, o.code, o.name)
    }

    /** A `lanlink://pair` link opened from elsewhere: ask before connecting. */
    fun offerPairFromLink(uri: String) {
        cancelPairing()
        parseOffer(uri)?.let { _pair.value = it }
    }

    fun acceptOffer() {
        val o = _pair.value as? PairState.Offer ?: return
        app.settings.onboarded = true
        startPairing(o.addresses, o.port, o.fingerprint, o.code, o.name)
    }

    private fun parseOffer(uri: String): PairState.Offer? = try {
        val p = JSONObject(NativeBridge.parsePairingUri(uri))
        PairState.Offer(
            name = p.optString("name"),
            addresses = p.getJSONArray("addresses").let { a -> List(a.length()) { a.getString(it) } },
            port = p.getInt("port"),
            fingerprint = p.getString("fingerprint"),
            code = p.getString("code"),
        )
    } catch (e: Exception) {
        _pair.value = PairState.Failed(e.message ?: "That is not a LANLink pairing code.")
        null
    }

    fun pairManual(address: String, port: Int, code: String) =
        startPairing(listOf(address.trim()), port, "", code, "")

    private fun startPairing(addresses: List<String>, port: Int, fingerprint: String, code: String, nameHint: String) {
        cancelPairing()
        pairJob = viewModelScope.launch(Dispatchers.IO) {
            val identity = try {
                app.identity.get(app.settings.deviceName)
            } catch (e: Exception) {
                _pair.value = PairState.Failed("Could not create this phone's identity: ${e.message}")
                return@launch
            }
            var lastError = "The PC did not answer."
            for (addr in addresses) {
                _pair.value = PairState.Working("Connecting to ${nameHint.ifEmpty { addr }}…")
                val h = try {
                    NativeBridge.pairStart(identity, app.settings.deviceName, android.os.Build.MODEL ?: "Android", addr, port, fingerprint, code)
                } catch (e: Exception) {
                    _pair.value = PairState.Failed(e.message ?: "Invalid pairing details.")
                    return@launch
                }
                pairHandle = h
                while (isActive) {
                    // Null only when the job was cancelled meanwhile.
                    val o = JSONObject(NativeBridge.pairPoll(h) ?: return@launch)
                    when (o.getString("state")) {
                        "connecting" -> {}
                        "confirm" -> _pair.value = PairState.Confirm(o.getString("code"))
                        "done" -> {
                            val host = PairedHost(
                                fingerprint = o.getString("fingerprint"),
                                name = o.optString("hostName").ifEmpty { nameHint.ifEmpty { "PC" } },
                                addresses = (listOf(addr) + addresses).distinct(),
                                port = port,
                                pairedAt = System.currentTimeMillis(),
                                lastConnected = 0,
                            )
                            app.hosts.put(host)
                            withContext(Dispatchers.Main) { refreshHosts() }
                            _pair.value = PairState.Done(host)
                            pairHandle = 0
                            return@launch
                        }
                        "failed" -> {
                            lastError = o.optString("error")
                            pairHandle = 0
                            break
                        }
                    }
                    delay(200)
                }
                // Try the next advertised address only for reachability
                // failures, never after the PC answered.
                if (!lastError.contains("reach") && !lastError.contains("did not answer")) break
            }
            _pair.value = PairState.Failed(friendlyPairError(lastError))
        }
    }

    private fun friendlyPairError(e: String): String = when {
        e.contains("wrong") -> "The code was not accepted. Check the 8-digit code on the PC and try again."
        e.contains("declined") -> "Pairing was declined on the PC."
        e.contains("not open") -> "Pairing is not open on the PC. Choose \"Pair a phone\" on the PC first."
        e.contains("identity mismatch") -> "The PC at that address is not the one in the QR code. Pairing stopped for your safety."
        e.contains("unverified") ->
            "That PC says it already knows this phone, but without its QR code LANLink cannot check that it is your PC. Scan the QR code on the PC instead."
        e.contains("did not answer") || e.contains("reach") ->
            "Cannot reach the PC. Make sure both are on the same Wi-Fi and Windows Firewall allows LANLink."
        else -> e
    }

    fun cancelPairing() {
        pairJob?.cancel()
        pairJob = null
        if (pairHandle != 0L) runCatching { NativeBridge.pairCancel(pairHandle) }
        pairHandle = 0
        _pair.value = PairState.Idle
    }

    // ------------------------------------------------------------ hosts

    fun forget(fp: String) {
        app.hosts.remove(fp)
        refreshHosts()
    }

    fun clearCredentials() {
        endSession()
        app.hosts.clear()
        app.identity.clear()
        refreshHosts()
    }

    // ------------------------------------------------------------ session

    fun connect(host: PairedHost, address: String? = null) {
        endSession()
        val s = RemoteSession(app, host, address)
        session = s
        _sessionVersion.value++
        s.start()
        go(Screen.Remote)
    }

    fun endSession() {
        session?.stop()
        session = null
        _sessionVersion.value++
        refreshHosts()
    }

    override fun onCleared() {
        endSession()
        cancelPairing()
        super.onCleared()
    }
}
