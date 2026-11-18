package com.lanlink.app.ui.screens

import android.view.HapticFeedbackConstants
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.lanlink.app.core.CompactLayout
import com.lanlink.app.core.KeyAction
import com.lanlink.app.core.KeySpec
import com.lanlink.app.core.ShiftState

private val KeyColor = Color(0xFF2A2F36)
private val KeyPressed = Color(0xFF4A525C)
private val KeyText = Color(0xFFE9E6DF)

/** Delay before a held key (Backspace, arrows) starts repeating, then the repeat interval. */
private const val REPEAT_DELAY_MS = 400L
private const val REPEAT_EVERY_MS = 55L

/**
 * LANLink's compact keyboard (see [CompactLayout]): about phone width,
 * centred, four short rows, so most of the PC stays visible above and
 * beside it. Keys act on press (no waiting for lift) and Backspace/arrows
 * repeat while held.
 */
@Composable
fun CompactKeyboard(
    keyHeight: Dp,
    sticky: Set<Int>,
    onText: (String) -> Unit,
    onKey: (Int) -> Unit,
    onSticky: (Int) -> Unit,
    onHide: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var symbols by remember { mutableStateOf(false) }
    val shift = remember { ShiftState() }
    var shiftVersion by remember { mutableIntStateOf(0) }
    // Read the version so a shift change recomposes the key labels.
    val shifted = shiftVersion.let { shift.shifted }
    // On small landscape screens or high display-zoom accessibility settings,
    // cap row height so the 4-row keyboard never covers more than ~42% of the screen.
    val screenH = LocalConfiguration.current.screenHeightDp.dp
    val effectiveKeyHeight = minOf(keyHeight, ((screenH * 0.42f - 25.dp) / 4f).coerceAtLeast(26.dp))

    fun press(k: KeySpec) {
        when (val a = k.action) {
            is KeyAction.Text -> {
                onText(a.text)
                if (!symbols) { shift.typed(); shiftVersion++ }
            }
            is KeyAction.Key -> onKey(a.vk)
            is KeyAction.Sticky -> onSticky(a.vk)
            KeyAction.Shift -> { shift.tap(System.currentTimeMillis()); shiftVersion++ }
            KeyAction.Layer -> symbols = !symbols
            KeyAction.Hide -> onHide()
        }
    }

    Box(modifier.fillMaxWidth(), contentAlignment = Alignment.BottomCenter) {
        Surface(
            color = Color(0xF215181C),
            shape = RoundedCornerShape(topStart = 18.dp, topEnd = 18.dp),
            shadowElevation = 10.dp,
            modifier = Modifier.widthIn(max = 640.dp).fillMaxWidth(),
        ) {
            Column(
                Modifier.navigationBarsPadding().padding(horizontal = 5.dp, vertical = 5.dp),
                verticalArrangement = Arrangement.spacedBy(5.dp),
            ) {
                CompactLayout.rows(symbols, shifted).forEach { row ->
                    Row(horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                        row.forEach { k ->
                            val active = when (val a = k.action) {
                                is KeyAction.Sticky -> a.vk in sticky
                                KeyAction.Shift -> shifted
                                else -> false
                            }
                            KeyCap(
                                spec = k,
                                active = active,
                                locked = k.action == KeyAction.Shift && shift.locked,
                                onPress = { press(k) },
                                modifier = Modifier.weight(k.weight).height(effectiveKeyHeight),
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun KeyCap(spec: KeySpec, active: Boolean, locked: Boolean, onPress: () -> Unit, modifier: Modifier) {
    val view = LocalView.current
    val press by rememberUpdatedState(onPress)
    var down by remember { mutableStateOf(false) }
    val bg = when {
        down -> KeyPressed
        active -> MaterialTheme.colorScheme.secondary
        else -> KeyColor
    }
    val fg = if (active && !down) Color(0xFF15181C) else KeyText
    val label = when {
        spec.action == KeyAction.Text(" ") -> "space"
        locked -> "⇪"
        else -> spec.label
    }
    Box(
        modifier
            .clip(RoundedCornerShape(8.dp))
            .background(bg)
            .semantics { contentDescription = if (label == "space") "Space" else label }
            .pointerInput(spec) {
                awaitEachGesture {
                    awaitFirstDown(requireUnconsumed = false)
                    down = true
                    view.performHapticFeedback(HapticFeedbackConstants.KEYBOARD_TAP)
                    press()
                    if (spec.repeats) {
                        // `true` once lifted or cancelled; null while still held.
                        var lifted = withTimeoutOrNull(REPEAT_DELAY_MS) { waitForUpOrCancellation(); true }
                        while (lifted == null) {
                            press()
                            lifted = withTimeoutOrNull(REPEAT_EVERY_MS) { waitForUpOrCancellation(); true }
                        }
                    } else {
                        waitForUpOrCancellation()
                    }
                    down = false
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        Text(
            label,
            color = if (label == "space") fg.copy(alpha = 0.5f) else fg,
            style = if (spec.label.length > 1) MaterialTheme.typography.labelMedium else MaterialTheme.typography.titleMedium,
            maxLines = 1,
        )
    }
}
