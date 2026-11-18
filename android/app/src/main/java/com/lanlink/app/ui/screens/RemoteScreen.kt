package com.lanlink.app.ui.screens

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.activity.compose.BackHandler
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import com.lanlink.app.core.KeyMap
import com.lanlink.app.core.PointerMode
import com.lanlink.app.core.Quality
import com.lanlink.app.session.Diagnostics
import com.lanlink.app.session.Phase
import com.lanlink.app.session.RemoteView
import com.lanlink.app.ui.MainActivity
import com.lanlink.app.ui.MainViewModel
import com.lanlink.app.ui.Screen
import com.lanlink.app.ui.theme.Glyphs
import kotlinx.coroutines.delay
import kotlin.math.roundToInt

private val Scrim = Color(0xCC15181C)
private val OnScrim = Color(0xFFE9E6DF)

@Composable
fun RemoteScreen(vm: MainViewModel, activity: MainActivity) {
    val session = vm.session
    if (session == null) {
        LaunchedEffect(Unit) { vm.back() }
        return
    }
    val st by session.state.collectAsState()
    val settings = vm.app.settings
    var panel by remember { mutableStateOf(false) }
    var poke by remember { mutableIntStateOf(0) }
    var zoom by remember { mutableFloatStateOf(1f) }
    var pointerMode by remember { mutableStateOf(settings.pointerMode) }
    var showStats by remember { mutableStateOf(settings.showStats) }
    var composing by remember { mutableStateOf("") }
    var view by remember { mutableStateOf<RemoteView?>(null) }
    val density = LocalDensity.current
    val imeBottom = WindowInsets.ime.getBottom(density)
    val imeVisible = imeBottom > 0
    // LANLink's compact keyboard (default) or the phone's own keyboard.
    var compactOpen by remember { mutableStateOf(false) }
    var compactPx by remember { mutableIntStateOf(0) }
    var barPx by remember { mutableIntStateOf(0) }
    val toggleKeyboard: () -> Unit = {
        if (settings.compactKeyboard) {
            if (imeVisible) view?.hideKeyboard()
            compactOpen = !compactOpen
        } else {
            compactOpen = false
            view?.toggleKeyboard()
        }
    }
    val toggleRef = rememberUpdatedState(toggleKeyboard)
    BackHandler(enabled = compactOpen) { compactOpen = false }
    // The picture keeps its size under the keyboard; the view slides so the
    // place being typed into stays visible (see RemoteView.setKeyboardInset).
    val covered = when {
        compactOpen -> compactPx
        imeVisible -> imeBottom + barPx
        else -> 0
    }
    LaunchedEffect(covered, view) { view?.setKeyboardInset(covered) }

    DisposableEffect(Unit) {
        activity.keepScreenOn(settings.keepScreenOn)
        // The PC is landscape: turning the phone sideways roughly doubles
        // the size of everything on it.
        if (settings.landscape) {
            activity.requestedOrientation = android.content.pm.ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
        }
        onDispose {
            activity.requestedOrientation = android.content.pm.ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
            activity.keepScreenOn(false)
            activity.remoteView = null
        }
    }
    // The control panel closes itself after a while without use.
    LaunchedEffect(panel, poke) {
        if (panel) { delay(8000); panel = false }
    }

    Box(Modifier.fillMaxSize().background(Color.Black)) {
        AndroidView(
            factory = { ctx ->
                RemoteView(ctx, session).also { rv ->
                    rv.pointerMode = settings.pointerMode
                    rv.setSensitivity(settings.sensitivity, settings.acceleration)
                    rv.initialZoom = if (settings.pointerMode == PointerMode.TRACKPAD) settings.startZoom else 1f
                    rv.fill = settings.fillScreen
                    rv.onZoomChanged = { zoom = it }
                    rv.onThreeFingerTap = { toggleRef.value() }
                    rv.keyboard.onComposing = { composing = it }
                    activity.remoteView = rv
                    view = rv
                }
            },
            modifier = Modifier.fillMaxSize(),
        )

        ControlRail(
            left = settings.controlsLeft,
            keyboardOpen = imeVisible || compactOpen,
            bottomInset = with(density) { if (compactOpen) compactPx.toDp() else 0.dp },
            pointerMode = pointerMode,
            zoom = zoom,
            moreOpen = panel,
            onKeyboard = { panel = false; toggleKeyboard() },
            onMode = {
                pointerMode = if (pointerMode == PointerMode.DIRECT) PointerMode.TRACKPAD else PointerMode.DIRECT
                settings.pointerMode = pointerMode
                view?.pointerMode = pointerMode
            },
            onZoom = {
                when {
                    zoom < 1.05f -> view?.zoomTo(2f)
                    zoom in 1.9f..2.1f -> view?.zoomActualPixels()
                    else -> view?.zoomFit()
                }
            },
            onMore = { panel = !panel; poke++ },
        ) {
            ControlPanel(
                hostName = st.hostName,
                pointerMode = pointerMode,
                zoom = zoom,
                displays = st.displays.map { it.id to it.name },
                currentDisplay = st.displayId,
                quality = settings.quality,
                onPoke = { poke++ },
                onMode = {
                    pointerMode = if (pointerMode == PointerMode.DIRECT) PointerMode.TRACKPAD else PointerMode.DIRECT
                    settings.pointerMode = pointerMode
                    view?.pointerMode = pointerMode
                },
                onZoomFit = { view?.zoomFit() },
                onZoom2 = { view?.zoomTo(2f) },
                onZoomActual = { view?.zoomActualPixels() },
                onDisplay = { session.selectDisplay(it) },
                onQuality = { settings.quality = it; session.applyQuality() },
                onStats = { showStats = !showStats; settings.showStats = showStats },
                onDiagnostics = { vm.go(Screen.Diagnostics) },
                onDisconnect = { vm.endSession(); vm.back() },
            )
        }

        if (showStats) {
            StatsOverlay(st.diagnostics, Modifier.align(Alignment.TopStart).safeDrawingPadding().padding(top = 56.dp, start = 8.dp))
        }

        st.notice?.let { n ->
            Surface(color = Scrim, shape = MaterialTheme.shapes.medium, modifier = Modifier.align(Alignment.BottomCenter).padding(bottom = 96.dp, start = 16.dp, end = 16.dp)) {
                Text(n, color = OnScrim, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(12.dp))
            }
        }

        if (compactOpen) {
            CompactKeyboard(
                keyHeight = settings.keyHeight.dp,
                sticky = st.modifiers,
                onText = { session.text(it) },
                onKey = { session.tapKey(it) },
                onSticky = { session.toggleSticky(it) },
                onHide = { compactOpen = false },
                modifier = Modifier.align(Alignment.BottomCenter).onSizeChanged { compactPx = it.height },
            )
        }

        if (imeVisible && !compactOpen) {
            Column(Modifier.align(Alignment.BottomCenter).fillMaxWidth().imePadding().onSizeChanged { barPx = it.height }) {
                if (composing.isNotEmpty()) {
                    Surface(color = Scrim, modifier = Modifier.padding(8.dp)) {
                        Text(composing, color = OnScrim, modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp))
                    }
                }
                ShortcutBar(st.modifiers, onToggle = { session.toggleSticky(it) }, onKey = { session.tapKey(it) }, onCombo = { session.combo(*it) })
            }
        }

        when (st.phase) {
            Phase.CONNECTING, Phase.STARTING, Phase.RECONNECTING -> StatusOverlay(
                title = when (st.phase) {
                    Phase.RECONNECTING -> "Reconnecting to ${st.hostName}…"
                    Phase.STARTING -> "Starting the stream…"
                    else -> "Connecting to ${st.hostName}…"
                },
                body = st.message,
                progress = true,
                primary = "Cancel" to { vm.endSession(); vm.back() },
            )
            Phase.FAILED -> StatusOverlay(
                title = "Not connected",
                body = st.message,
                progress = false,
                primary = "Try again" to { session.retry() },
                secondary = "Close" to { vm.endSession(); vm.back() },
            )
            Phase.ENDED -> StatusOverlay(
                title = "Session ended",
                body = st.message,
                progress = false,
                primary = "Reconnect" to { session.retry() },
                secondary = "Close" to { vm.endSession(); vm.back() },
            )
            Phase.STREAMING -> {}
        }
    }
}

/**
 * Controls docked to one edge of the screen (right by default: under the
 * right thumb, and on the far side from the camera cutout in the usual
 * landscape orientation). After a few seconds without use the rail slides
 * away to a thin tab; tapping the tab brings it back. It stays out while
 * the keyboard or the "more" panel is open, so both can always be closed.
 */
@Composable
private fun ControlRail(
    left: Boolean,
    keyboardOpen: Boolean,
    bottomInset: androidx.compose.ui.unit.Dp,
    pointerMode: PointerMode,
    zoom: Float,
    moreOpen: Boolean,
    onKeyboard: () -> Unit,
    onMode: () -> Unit,
    onZoom: () -> Unit,
    onMore: () -> Unit,
    more: @Composable () -> Unit,
) {
    var shown by remember { mutableStateOf(true) }
    var poke by remember { mutableIntStateOf(0) }
    val out = shown || moreOpen || keyboardOpen
    LaunchedEffect(shown, poke, moreOpen, keyboardOpen) {
        if (shown && !moreOpen && !keyboardOpen) { delay(4000); shown = false }
    }
    val edge = if (left) Alignment.CenterStart else Alignment.CenterEnd
    Box(Modifier.fillMaxSize().safeDrawingPadding().imePadding().padding(bottom = bottomInset)) {
        if (moreOpen) {
            // Tap anywhere outside the panel to close it.
            Box(
                Modifier.fillMaxSize().clickable(
                    interactionSource = remember { MutableInteractionSource() }, indication = null,
                ) { onMore() },
            )
            Box(Modifier.align(edge).padding(horizontal = 76.dp)) { more() }
        }
        AnimatedVisibility(
            visible = out,
            modifier = Modifier.align(edge),
            enter = slideInHorizontally { if (left) -it else it } + fadeIn(),
            exit = slideOutHorizontally { if (left) -it else it } + fadeOut(),
        ) {
            Surface(
                color = Scrim,
                shape = RoundedCornerShape(26.dp),
                shadowElevation = 6.dp,
                modifier = Modifier.padding(horizontal = 8.dp),
            ) {
                Column(
                    Modifier.padding(vertical = 6.dp).verticalScroll(rememberScrollState()),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(2.dp),
                ) {
                    RailButton(Glyphs.Keyboard, if (keyboardOpen) "Hide keyboard" else "Show keyboard", active = keyboardOpen) { poke++; onKeyboard() }
                    val trackpad = pointerMode == PointerMode.TRACKPAD
                    RailButton(
                        if (trackpad) Glyphs.Mouse else Glyphs.Touch,
                        if (trackpad) "Trackpad mode (tap for touch)" else "Touch mode (tap for trackpad)",
                    ) { poke++; onMode() }
                    RailLabel(if (zoom < 1.05f) "Fit" else "%.1f×".format(zoom), "Zoom: Fit, 2×, 1:1") { poke++; onZoom() }
                    RailButton(Glyphs.Menu, "More controls", active = moreOpen) { poke++; onMore() }
                }
            }
        }
        if (!out) {
            Surface(
                color = Scrim,
                shape = if (left) RoundedCornerShape(topEnd = 12.dp, bottomEnd = 12.dp)
                else RoundedCornerShape(topStart = 12.dp, bottomStart = 12.dp),
                onClick = { shown = true; poke++ },
                modifier = Modifier.align(edge).width(20.dp).heightIn(min = 76.dp)
                    .semantics { contentDescription = "Show controls" },
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Box(Modifier.width(3.dp).heightIn(min = 30.dp).background(OnScrim.copy(alpha = 0.6f), RoundedCornerShape(2.dp)))
                }
            }
        }
    }
}

@Composable
private fun RailButton(icon: ImageVector, label: String, active: Boolean = false, onClick: () -> Unit) {
    Surface(
        onClick = onClick,
        color = if (active) MaterialTheme.colorScheme.secondary else Color.Transparent,
        contentColor = if (active) Color(0xFF15181C) else OnScrim,
        shape = CircleShape,
        modifier = Modifier.padding(horizontal = 4.dp).size(48.dp),
    ) {
        Box(contentAlignment = Alignment.Center) {
            Icon(icon, contentDescription = label, modifier = Modifier.size(22.dp))
        }
    }
}

@Composable
private fun RailLabel(text: String, label: String, onClick: () -> Unit) {
    Surface(
        onClick = onClick,
        color = Color.Transparent,
        contentColor = OnScrim,
        shape = CircleShape,
        modifier = Modifier.padding(horizontal = 4.dp).size(48.dp).semantics { contentDescription = label },
    ) {
        Box(contentAlignment = Alignment.Center) {
            Text(text, style = MaterialTheme.typography.labelLarge, maxLines = 1)
        }
    }
}

@Composable
private fun ControlPanel(
    hostName: String,
    pointerMode: PointerMode,
    zoom: Float,
    displays: List<Pair<Long, String>>,
    currentDisplay: Long,
    quality: Quality,
    onPoke: () -> Unit,
    onMode: () -> Unit,
    onZoomFit: () -> Unit,
    onZoom2: () -> Unit,
    onZoomActual: () -> Unit,
    onDisplay: (Long) -> Unit,
    onQuality: (Quality) -> Unit,
    onStats: () -> Unit,
    onDiagnostics: () -> Unit,
    onDisconnect: () -> Unit,
) {
    var displayMenu by remember { mutableStateOf(false) }
    var qualityMenu by remember { mutableStateOf(false) }
    Surface(color = Scrim, shape = RoundedCornerShape(20.dp), shadowElevation = 8.dp) {
        Column(Modifier.padding(10.dp).widthIn(max = 300.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(hostName, color = OnScrim, style = MaterialTheme.typography.labelLarge, maxLines = 1, modifier = Modifier.padding(start = 6.dp, top = 2.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                PanelChip(Glyphs.Mouse, "Trackpad", pointerMode == PointerMode.TRACKPAD) { onPoke(); if (pointerMode != PointerMode.TRACKPAD) onMode() }
                PanelChip(Glyphs.Touch, "Touch", pointerMode == PointerMode.DIRECT) { onPoke(); if (pointerMode != PointerMode.DIRECT) onMode() }
            }
            Text("Zoom ${"%.1f".format(zoom)}×", color = OnScrim.copy(alpha = 0.7f), style = MaterialTheme.typography.labelMedium, modifier = Modifier.padding(start = 6.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                PanelChip(null, "Fit", zoom < 1.05f) { onPoke(); onZoomFit() }
                PanelChip(null, "2×", zoom in 1.9f..2.1f) { onPoke(); onZoom2() }
                PanelChip(null, "1:1", false) { onPoke(); onZoomActual() }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(2.dp), verticalAlignment = Alignment.CenterVertically) {
                if (displays.size > 1) {
                    Box {
                        ToolButton(Glyphs.Monitor, "Switch display") { onPoke(); displayMenu = true }
                        DropdownMenu(displayMenu, onDismissRequest = { displayMenu = false }) {
                            displays.forEach { (id, name) ->
                                DropdownMenuItem(text = { Text((if (id == currentDisplay) "✓ " else "") + name) }, onClick = { displayMenu = false; onDisplay(id) })
                            }
                        }
                    }
                }
                Box {
                    ToolButton(Glyphs.Quality, "Stream quality") { onPoke(); qualityMenu = true }
                    DropdownMenu(qualityMenu, onDismissRequest = { qualityMenu = false }) {
                        listOf(Quality.SMOOTH, Quality.BALANCED, Quality.SHARP).forEach { q ->
                            DropdownMenuItem(text = { Text((if (q == quality) "✓ " else "") + q.label) }, onClick = { qualityMenu = false; onQuality(q) })
                        }
                        DropdownMenuItem(text = { Text("Diagnostics…") }, onClick = { qualityMenu = false; onDiagnostics() })
                    }
                }
                ToolButton(Glyphs.Gauge, "Performance overlay") { onPoke(); onStats() }
                ToolButton(Glyphs.Power, "Disconnect", tint = Color(0xFFE5A39B)) { onDisconnect() }
            }
        }
    }
}

@Composable
private fun PanelChip(icon: ImageVector?, label: String, selected: Boolean, onClick: () -> Unit) {
    Surface(
        onClick = onClick,
        color = if (selected) MaterialTheme.colorScheme.secondary else Color(0x26FFFFFF),
        contentColor = if (selected) Color(0xFF15181C) else OnScrim,
        shape = RoundedCornerShape(12.dp),
        modifier = Modifier.heightIn(min = 44.dp),
    ) {
        Row(Modifier.padding(horizontal = 14.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            if (icon != null) Icon(icon, contentDescription = null, modifier = Modifier.size(18.dp))
            Text(label, style = MaterialTheme.typography.labelLarge, modifier = Modifier.padding(vertical = 12.dp))
        }
    }
}

@Composable
private fun ToolButton(icon: ImageVector, label: String, tint: Color = OnScrim, onClick: () -> Unit) {
    IconButton(onClick = onClick, modifier = Modifier.size(48.dp)) {
        Icon(icon, contentDescription = label, tint = tint, modifier = Modifier.size(22.dp))
    }
}

@Composable
private fun ShortcutBar(active: Set<Int>, onToggle: (Int) -> Unit, onKey: (Int) -> Unit, onCombo: (IntArray) -> Unit) {
    var fn by remember { mutableStateOf(false) }
    Surface(color = Scrim, modifier = Modifier.fillMaxWidth()) {
        Row(
            Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 6.dp, vertical = 4.dp),
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            KeyChip("Esc") { onKey(KeyMap.VK_ESCAPE) }
            KeyChip("Tab") { onKey(KeyMap.VK_TAB) }
            KeyChip("Ctrl", KeyMap.VK_LCONTROL in active) { onToggle(KeyMap.VK_LCONTROL) }
            KeyChip("Alt", KeyMap.VK_LMENU in active) { onToggle(KeyMap.VK_LMENU) }
            KeyChip("Shift", KeyMap.VK_LSHIFT in active) { onToggle(KeyMap.VK_LSHIFT) }
            KeyChip("Win", KeyMap.VK_LWIN in active) { onToggle(KeyMap.VK_LWIN) }
            KeyChip("←") { onKey(KeyMap.VK_LEFT) }
            KeyChip("↑") { onKey(KeyMap.VK_UP) }
            KeyChip("↓") { onKey(KeyMap.VK_DOWN) }
            KeyChip("→") { onKey(KeyMap.VK_RIGHT) }
            KeyChip("Del") { onKey(KeyMap.VK_DELETE) }
            KeyChip("Fn", fn) { fn = !fn }
            if (fn) for (i in 0 until 12) KeyChip("F${i + 1}") { onKey(KeyMap.VK_F1 + i) }
            KeyChip("Alt+Tab") { onCombo(intArrayOf(KeyMap.VK_LMENU, KeyMap.VK_TAB)) }
            KeyChip("Win+D") { onCombo(intArrayOf(KeyMap.VK_LWIN, 0x44)) }
            KeyChip("Ctrl+C") { onCombo(intArrayOf(KeyMap.VK_LCONTROL, 0x43)) }
            KeyChip("Ctrl+V") { onCombo(intArrayOf(KeyMap.VK_LCONTROL, 0x56)) }
            KeyChip("Ctrl+Z") { onCombo(intArrayOf(KeyMap.VK_LCONTROL, 0x5A)) }
            KeyChip("Home") { onKey(KeyMap.VK_HOME) }
            KeyChip("End") { onKey(KeyMap.VK_END) }
            KeyChip("PgUp") { onKey(KeyMap.VK_PRIOR) }
            KeyChip("PgDn") { onKey(KeyMap.VK_NEXT) }
        }
    }
}

@Composable
private fun KeyChip(label: String, active: Boolean = false, onClick: () -> Unit) {
    Surface(
        onClick = onClick,
        color = if (active) MaterialTheme.colorScheme.secondary else Color(0x33FFFFFF),
        contentColor = if (active) Color(0xFF15181C) else OnScrim,
        shape = RoundedCornerShape(8.dp),
        modifier = Modifier.heightIn(min = 40.dp),
    ) {
        Box(Modifier.padding(horizontal = 12.dp), contentAlignment = Alignment.Center) {
            Text(label, style = MaterialTheme.typography.labelLarge, modifier = Modifier.padding(vertical = 10.dp))
        }
    }
}

@Composable
private fun StatsOverlay(d: Diagnostics, modifier: Modifier) {
    fun f(v: Float?, unit: String, digits: Int = 0) = v?.let { "%.${digits}f$unit".format(it) } ?: "–"
    Surface(color = Color(0xAA000000), shape = RoundedCornerShape(8.dp), modifier = modifier) {
        Column(Modifier.padding(8.dp)) {
            val mono = MaterialTheme.typography.labelMedium.copy(fontFamily = FontFamily.Monospace, fontSize = 11.sp, color = OnScrim)
            Text("rtt ${f(d.rttMs, " ms", 1)}   fps ${f(d.renderedFps, "")}", style = mono)
            Text("${d.resolution ?: "–"}  ${d.bitrateKbps?.let { "${it / 1000.0}".take(4) + " Mb/s" } ?: "–"}", style = mono)
            Text("enc ${f(d.encodeP50Ms, " ms", 1)}  dec ${f(d.decodeP50Ms, " ms", 1)}", style = mono)
            Text(
                "glass≈ " + (d.latencyEstimateMs?.let { "%.0f±%.0f ms".format(it, d.latencyUncertaintyMs ?: 0f) } ?: "–"),
                style = mono,
            )
        }
    }
}

@Composable
private fun StatusOverlay(
    title: String,
    body: String?,
    progress: Boolean,
    primary: Pair<String, () -> Unit>,
    secondary: Pair<String, () -> Unit>? = null,
) {
    com.lanlink.app.ui.theme.LanLinkTheme(dark = true) {
    Box(Modifier.fillMaxSize().background(Color(0xE615181C)).safeDrawingPadding(), contentAlignment = Alignment.Center) {
        Column(Modifier.padding(28.dp).widthIn(max = 420.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            if (progress) CircularProgressIndicator(Modifier.size(28.dp), color = OnScrim, strokeWidth = 2.5.dp)
            Text(title, color = OnScrim, style = MaterialTheme.typography.titleLarge)
            if (body != null) Text(body, color = OnScrim.copy(alpha = 0.8f), style = MaterialTheme.typography.bodyMedium)
            Spacer(Modifier.size(4.dp))
            PrimaryButton(primary.first, primary.second)
            if (secondary != null) SecondaryButton(secondary.first, secondary.second)
        }
    }
}
}
