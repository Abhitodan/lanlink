package com.lanlink.app.ui.screens

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import com.lanlink.app.ui.MainActivity
import com.lanlink.app.ui.MainViewModel
import com.lanlink.app.ui.Screen

@Composable
fun AppRoot(vm: MainViewModel, activity: MainActivity) {
    val screen by vm.screen.collectAsState()
    AnimatedContent(
        targetState = screen,
        transitionSpec = { fadeIn() togetherWith fadeOut() },
        contentKey = { it::class },
        label = "screen",
    ) { s ->
        when (s) {
            Screen.Onboarding -> OnboardingScreen(vm)
            Screen.Home -> HomeScreen(vm)
            is Screen.Pair -> PairScreen(vm, s)
            is Screen.Host -> HostScreen(vm, s.fingerprint)
            Screen.Remote -> RemoteScreen(vm, activity)
            Screen.Settings -> SettingsScreen(vm)
            Screen.Diagnostics -> DiagnosticsScreen(vm)
            Screen.Security -> SecurityScreen(vm)
        }
    }
}
