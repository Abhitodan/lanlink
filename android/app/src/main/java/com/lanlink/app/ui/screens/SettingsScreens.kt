package com.lanlink.app.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import com.lanlink.app.core.PointerMode
import com.lanlink.app.core.Quality
import com.lanlink.app.session.SessionState
import com.lanlink.app.ui.MainViewModel
import com.lanlink.app.ui.Screen
import com.lanlink.app.ui.theme.Glyphs

@Composable
fun SettingsScreen(vm: MainViewModel) {
    val s = vm.app.settings
    var quality by remember { mutableStateOf(s.quality) }
    var mode by remember { mutableStateOf(s.pointerMode) }
    var sens by remember { mutableFloatStateOf(s.sensitivity) }
    var accel by remember { mutableFloatStateOf(s.acceleration) }
    var fill by remember { mutableStateOf(s.fillScreen) }
    var landscape by remember { mutableStateOf(s.landscape) }
    var controlsLeft by remember { mutableStateOf(s.controlsLeft) }
    var allowShots by remember { mutableStateOf(s.allowScreenshots) }
    var compactKeyboard by remember { mutableStateOf(s.compactKeyboard) }
    var keyHeight by remember { mutableFloatStateOf(s.keyHeight) }
    var startZoom by remember { mutableFloatStateOf(s.startZoom) }
    var awake by remember { mutableStateOf(s.keepScreenOn) }
    var stats by remember { mutableStateOf(s.showStats) }
    var viewOnly by remember { mutableStateOf(s.viewOnly) }
    var fps by remember { mutableStateOf(if (s.customFps == 0) "" else s.customFps.toString()) }
    var kbps by remember { mutableStateOf(if (s.customKbps == 0) "" else s.customKbps.toString()) }
    var maxW by remember { mutableStateOf(if (s.customMaxWidth == 0) "" else s.customMaxWidth.toString()) }
    var cap by remember { mutableStateOf(if (s.maxKbps == 0) "" else s.maxKbps.toString()) }
    var name by remember { mutableStateOf(s.deviceName) }

    fun applyLive() = vm.session?.applyQuality()

    Page("Settings", onBack = { vm.back() }) {
        SectionLabel("Stream quality")
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Quality.entries.forEach { q ->
                FilterChip(selected = quality == q, onClick = { quality = q; s.quality = q; applyLive() }, label = { Text(q.label) })
            }
        }
        Note(
            when (quality) {
                Quality.SMOOTH -> "Prioritises responsiveness and steady motion; lower resolution."
                Quality.BALANCED -> "Clear text and smooth motion at the phone's screen resolution."
                Quality.SHARP -> "Full PC resolution for fine text, especially when zooming in. Uses more bandwidth."
                Quality.CUSTOM -> "Set your own limits. Empty fields are decided automatically."
            },
        )
        if (quality == Quality.CUSTOM) {
            NumberField("Frame rate (fps)", fps) { fps = it; s.customFps = it.toIntOrNull() ?: 0 }
            NumberField("Bitrate (kbps)", kbps) { kbps = it; s.customKbps = it.toIntOrNull() ?: 0 }
            NumberField("Maximum width (px)", maxW) { maxW = it; s.customMaxWidth = it.toIntOrNull() ?: 0 }
            PrimaryButton("Apply to current session", { applyLive() })
        }
        NumberField("Bandwidth limit (kbps, optional)", cap) { cap = it; s.maxKbps = it.toIntOrNull() ?: 0; applyLive() }

        SectionLabel("Touch and pointer")
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            FilterChip(selected = mode == PointerMode.DIRECT, onClick = { mode = PointerMode.DIRECT; s.pointerMode = mode },
                label = { Text("Direct touch") }, leadingIcon = { Icon(Glyphs.Touch, null) })
            FilterChip(selected = mode == PointerMode.TRACKPAD, onClick = { mode = PointerMode.TRACKPAD; s.pointerMode = mode },
                label = { Text("Trackpad") }, leadingIcon = { Icon(Glyphs.Mouse, null) })
        }
        Note(
            if (mode == PointerMode.DIRECT) "Tap to click. Long-press to drag. Two fingers scroll; pinch to zoom; two-finger tap for right click."
            else "Slide to move the pointer. Tap to click; double-tap and hold to drag. Two fingers scroll; two-finger tap for right click; three-finger tap for the keyboard.",
        )
        Text("Trackpad sensitivity: ${"%.1f".format(sens)}×", style = MaterialTheme.typography.bodyMedium)
        Slider(sens, { sens = it; s.sensitivity = it }, valueRange = 0.3f..3f)
        Text("Pointer acceleration: ${"%.1f".format(accel)}", style = MaterialTheme.typography.bodyMedium)
        Slider(accel, { accel = it; s.acceleration = it }, valueRange = 0f..2f)
        Text("Trackpad starts zoomed in: ${"%.1f".format(startZoom)}×", style = MaterialTheme.typography.bodyMedium)
        Slider(startZoom, { startZoom = it; s.startZoom = it }, valueRange = 1f..4f)
        Note("The view follows the pointer when it nears the edge, so you never have to pan by hand.")

        SectionLabel("Display")
        Toggle("Turn sideways while controlling", landscape) { landscape = it; s.landscape = it }
        Toggle("Controls on the left edge", controlsLeft) { controlsLeft = it; s.controlsLeft = it }

        SectionLabel("Keyboard")
        Toggle("Compact LANLink keyboard", compactKeyboard) { compactKeyboard = it; s.compactKeyboard = it }
        if (compactKeyboard) {
            Text("Key size: ${keyHeight.toInt()} dp", style = MaterialTheme.typography.bodyMedium)
            Slider(keyHeight, { keyHeight = it; s.keyHeight = it }, valueRange = 28f..48f)
            Note("Phone-width with Esc, Tab, Ctrl, Alt and arrows built in. Turn off to use your phone's own keyboard (swipe typing, other languages).")
        }
        Toggle("Fill the screen (crop edges)", fill) { fill = it; s.fillScreen = it }
        Toggle("Keep phone screen on during a session", awake) { awake = it; s.keepScreenOn = it }
        Toggle("Show performance overlay", stats) { stats = it; s.showStats = it }
        Toggle("View only (do not control the PC)", viewOnly) { viewOnly = it; s.viewOnly = it }

        SectionLabel("Privacy")
        Toggle("Allow screenshots of the PC screen", allowShots) { allowShots = it; s.allowScreenshots = it }
        Note("Off: the PC screen stays out of screenshots, screen recordings and the recent-apps view.")

        SectionLabel("This phone")
        OutlinedTextField(name, { name = it.take(48); s.deviceName = name }, Modifier.fillMaxWidth(), label = { Text("Name shown on the PC") }, singleLine = true)

        SectionLabel("More")
        SecondaryButton("Security and paired PCs", { vm.go(Screen.Security) }, icon = Glyphs.Shield)
        SecondaryButton("Performance diagnostics", { vm.go(Screen.Diagnostics) }, icon = Glyphs.Gauge)
        Note("Audio and clipboard sharing are not available in this version.")
    }
}

@Composable
private fun NumberField(label: String, value: String, onChange: (String) -> Unit) {
    OutlinedTextField(
        value, { v -> onChange(v.filter { it.isDigit() }.take(6)) }, Modifier.fillMaxWidth(),
        label = { Text(label) }, singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
    )
}

@Composable
fun Toggle(label: String, value: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 48.dp).semantics { role = Role.Switch },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
        Switch(value, onChange)
    }
}

@Composable
fun DiagnosticsScreen(vm: MainViewModel) {
    val session = vm.session
    val st: SessionState? = session?.state?.collectAsState()?.value
    Page("Performance", onBack = { vm.back() }) {
        if (st == null) {
            Note("Connect to a PC to see live measurements.")
            return@Page
        }
        val d = st.diagnostics
        fun ms(v: Float?) = v?.let { "%.1f ms".format(it) }
        fun fps(v: Float?) = v?.let { "%.0f fps".format(it) }
        SectionLabel("Network")
        Card {
            MetricRow("Round trip", ms(d.rttMs), "Ping to the PC and back")
            MetricRow("Video bitrate", d.bitrateKbps?.let { "%.1f Mb/s".format(it / 1000.0) })
            MetricRow("Target bitrate", d.targetKbps?.let { "%.1f Mb/s".format(it / 1000.0) }, "Adapts to network conditions")
            MetricRow("Packets lost (total)", d.lostPackets?.toString())
            MetricRow("Reconnections", st.reconnects.toString())
        }
        SectionLabel("PC")
        Card {
            MetricRow("Captured", fps(d.capturedFps), "Changes on the PC screen; a still screen sends no frames")
            MetricRow("Encoded", fps(d.encodedFps))
            MetricRow("Encode time (median / 95%)", d.encodeP50Ms?.let { "%.1f / %.1f ms".format(it, d.encodeP95Ms ?: 0f) })
            MetricRow("Capture → sent", ms(d.hostPipelineMs), "Measured on the PC's own clock")
            MetricRow("Frames dropped", d.droppedOnPc?.toString(), "Last second")
            MetricRow("Send queue", d.queueDepth?.toString())
            MetricRow("Encoder", st.encoder.ifEmpty { null })
            MetricRow("Capture", st.capture.ifEmpty { null })
        }
        SectionLabel("Phone")
        Card {
            MetricRow("Received", fps(d.receivedFps))
            MetricRow("Displayed", fps(d.renderedFps))
            MetricRow("Decode time (median / 95%)", d.decodeP50Ms?.let { "%.1f / %.1f ms".format(it, d.decodeP95Ms ?: 0f) })
            MetricRow("Frames skipped", d.skippedOnPhone?.toString(), "Waiting for a keyframe after loss")
            MetricRow("Resolution", d.resolution)
            MetricRow("Decoder", d.decoder)
        }
        SectionLabel("End to end")
        Card {
            MetricRow(
                "PC capture → phone decoded",
                d.latencyEstimateMs?.let { "≈ %.0f ms ± %.0f".format(it, d.latencyUncertaintyMs ?: 0f) },
                "Estimated: the two clocks are aligned using the round trip, so the uncertainty is half the round trip.",
            )
            MetricRow("Touch → visible on phone", null, "Needs a high-speed camera; not measured in the app")
        }
    }
}

@Composable
fun SecurityScreen(vm: MainViewModel) {
    val hosts by vm.hosts.collectAsState()
    var confirmClear by remember { mutableStateOf(false) }
    val fp = remember { runCatching { vm.app.identity.fingerprint(vm.app.settings.deviceName) }.getOrNull() }
    Page("Security", onBack = { vm.back() }) {
        SectionLabel("This phone's identity")
        Card {
            Text(fp?.take(16)?.uppercase()?.chunked(4)?.joinToString("-") ?: "Not created yet", style = MaterialTheme.typography.titleMedium)
            Note("PCs recognise this phone by this identity. Its private key is protected by the Android Keystore and never leaves the phone.")
        }
        SectionLabel("Paired PCs")
        if (hosts.isEmpty()) Note("No paired PCs.")
        hosts.forEach { h ->
            Card {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(h.name, style = MaterialTheme.typography.titleMedium)
                        Text(h.fingerprint.take(16).uppercase().chunked(4).joinToString("-"), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    TextButton(onClick = { vm.forget(h.fingerprint) }) { Text("Forget", color = MaterialTheme.colorScheme.error) }
                }
            }
        }
        SectionLabel("Sessions")
        Note("Every session is encrypted end to end on your network. The PC shows when it is being controlled and can stop the session at any time. To require approval on the PC for every connection, enable it in LANLink on the PC.")
        Note("Clipboard sharing is not implemented in this version, so nothing you copy is sent between devices.")
        SectionLabel("Reset")
        SecondaryButton("Remove identity and all paired PCs", { confirmClear = true }, danger = true)
    }
    if (confirmClear) {
        AlertDialog(
            onDismissRequest = { confirmClear = false },
            title = { Text("Remove everything?") },
            text = { Text("This phone will forget all PCs and create a new identity. You will need to pair again, and PCs will see this phone as new.") },
            confirmButton = { TextButton(onClick = { confirmClear = false; vm.clearCredentials() }) { Text("Remove", color = MaterialTheme.colorScheme.error) } },
            dismissButton = { TextButton(onClick = { confirmClear = false }) { Text("Cancel") } },
        )
    }
}
