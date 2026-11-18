package com.lanlink.app.ui.screens

import android.text.format.DateUtils
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.lanlink.app.core.FoundHost
import com.lanlink.app.core.PairedHost
import com.lanlink.app.ui.MainViewModel
import com.lanlink.app.ui.Screen
import com.lanlink.app.ui.theme.Glyphs
import com.lanlink.app.ui.theme.Palette

@Composable
fun OnboardingScreen(vm: MainViewModel) {
    Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        Column(
            Modifier.safeDrawingPadding().padding(28.dp).fillMaxSize(),
            verticalArrangement = Arrangement.spacedBy(18.dp),
        ) {
            Spacer(Modifier.weight(1f))
            Icon(Glyphs.Laptop, null, Modifier.size(56.dp), tint = MaterialTheme.colorScheme.primary)
            Text("Your PC, in your hand.", style = MaterialTheme.typography.headlineMedium)
            Text(
                "LANLink shows your Windows screen on this phone and lets you use it with touch and the keyboard, over your own Wi-Fi.",
                style = MaterialTheme.typography.bodyLarge,
            )
            Card {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(Glyphs.Shield, null, Modifier.size(22.dp), tint = MaterialTheme.colorScheme.primary)
                    Spacer(Modifier.width(12.dp))
                    Text("Private by design", style = MaterialTheme.typography.titleMedium)
                }
                Note(
                    "Everything stays on your local network and is encrypted. Your PC must approve this phone once, " +
                        "and can remove it at any time. No account, no cloud.",
                )
            }
            Spacer(Modifier.weight(1f))
            PrimaryButton("Scan the PC's pairing code", { vm.finishOnboarding(Screen.Pair()) }, icon = Glyphs.QrScan)
            SecondaryButton("Find my PC on this network", { vm.finishOnboarding(Screen.Home) }, icon = Glyphs.Laptop)
            Note("On the PC, run LANLink and choose \"Pair a phone\" to show the code.")
        }
    }
}

@Composable
fun HomeScreen(vm: MainViewModel) {
    val hosts by vm.hosts.collectAsState()
    val found by vm.app.discovery.hosts.collectAsState()
    val status by vm.app.discovery.status.collectAsState()

    // Browse only while this screen is visible.
    DisposableEffect(Unit) {
        vm.refreshHosts()
        vm.app.discovery.start()
        onDispose { vm.app.discovery.stop() }
    }

    Page(
        title = "LANLink",
        onBack = null,
        actions = {
            IconButton(onClick = { vm.go(Screen.Settings) }) { Icon(Icons.Filled.Settings, contentDescription = "Settings") }
        },
    ) {
        if (hosts.isNotEmpty()) {
            SectionLabel("Your PCs")
            hosts.forEach { h ->
                val live = found.firstOrNull { it.shortId.isNotEmpty() && it.shortId == h.shortId }
                PairedHostCard(h, live, onOpen = { vm.go(Screen.Host(h.fingerprint)) }, onConnect = { vm.connect(h, live?.address) })
            }
        }

        val unpaired = found.filter { f -> hosts.none { it.shortId == f.shortId } }
        SectionLabel("On this network")
        when {
            unpaired.isNotEmpty() -> unpaired.forEach { f ->
                Card(onClick = { vm.go(Screen.Pair(prefillAddress = f.address, prefillPort = f.port)) }) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(Glyphs.Laptop, null, Modifier.size(22.dp))
                        Spacer(Modifier.width(12.dp))
                        Column(Modifier.weight(1f)) {
                            Text(f.name, style = MaterialTheme.typography.titleMedium)
                            Text("Not paired · tap to pair", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                    }
                }
            }
            status.startsWith("failed") -> Note(
                "Automatic discovery is not available on this network (some routers block it). " +
                    "You can still pair by scanning the QR code or entering the PC's address.",
            )
            else -> Row(verticalAlignment = Alignment.CenterVertically) {
                CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                Spacer(Modifier.width(12.dp))
                Note(if (hosts.isEmpty()) "Looking for PCs running LANLink…" else "Looking for other PCs…")
            }
        }

        PrimaryButton("Pair a PC", { vm.go(Screen.Pair()) }, icon = Icons.Filled.Add)
        Note("Not seeing your PC? Both devices must be on the same Wi-Fi, and guest networks often block devices from seeing each other.")
    }
}

@Composable
private fun PairedHostCard(h: PairedHost, live: FoundHost?, onOpen: () -> Unit, onConnect: () -> Unit) {
    Card(onClick = onOpen) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(Glyphs.Laptop, null, Modifier.size(24.dp), tint = MaterialTheme.colorScheme.primary)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(h.name, style = MaterialTheme.typography.titleMedium)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    StatusDot(if (live != null) Palette.Ok else MaterialTheme.colorScheme.outline)
                    Spacer(Modifier.width(6.dp))
                    Text(
                        if (live != null) "Available" else lastSeen(h),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            androidx.compose.material3.FilledTonalButton(onClick = onConnect) { Text("Connect") }
        }
    }
}

fun lastSeen(h: PairedHost): String =
    if (h.lastConnected <= 0) "Paired, not connected yet"
    else "Last used " + DateUtils.getRelativeTimeSpanString(h.lastConnected, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS)

@Composable
fun HostScreen(vm: MainViewModel, fingerprint: String) {
    val hosts by vm.hosts.collectAsState()
    val h = hosts.firstOrNull { it.fingerprint == fingerprint }
    if (h == null) {
        Page("PC", onBack = { vm.back() }) { Note("This PC is no longer paired.") }
        return
    }
    val found by vm.app.discovery.hosts.collectAsState()
    val live = found.firstOrNull { it.shortId == h.shortId }
    Page(h.name, onBack = { vm.back() }) {
        Card {
            Row(verticalAlignment = Alignment.CenterVertically) {
                StatusDot(if (live != null) Palette.Ok else MaterialTheme.colorScheme.outline)
                Spacer(Modifier.width(8.dp))
                Text(if (live != null) "Available on this network" else "Not seen on this network right now", style = MaterialTheme.typography.bodyMedium)
            }
            Text(lastSeen(h), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        PrimaryButton("Connect", { vm.connect(h, live?.address) }, icon = Glyphs.Monitor)
        SectionLabel("Details")
        Card {
            MetricRow("Identity", h.fingerprint.take(16).uppercase().chunked(4).joinToString("-"))
            MetricRow("Addresses", (listOfNotNull(live?.address) + h.addresses).distinct().joinToString(", "))
            MetricRow("Port", h.port.toString())
        }
        Note("Displays and stream quality can be changed while connected.")
        SectionLabel("Manage")
        SecondaryButton("Connection settings", { vm.go(Screen.Settings) }, icon = Glyphs.Quality)
        SecondaryButton("Forget this PC", { vm.forget(h.fingerprint); vm.back() }, danger = true)
        Note("Forgetting removes the PC from this phone. To stop the PC trusting this phone, remove it on the PC as well.")
    }
}

@Composable
fun CenteredProgress(text: String) {
    Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
            Spacer(Modifier.width(12.dp))
            Text(text, style = MaterialTheme.typography.bodyMedium)
        }
    }
}
