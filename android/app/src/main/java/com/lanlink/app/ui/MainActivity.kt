package com.lanlink.app.ui

import android.content.Intent
import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import com.lanlink.app.session.RemoteView
import com.lanlink.app.ui.screens.AppRoot
import com.lanlink.app.ui.theme.LanLinkTheme

class MainActivity : ComponentActivity() {
    private val vm: MainViewModel by viewModels()

    /** The live remote view, for hardware keyboard routing. */
    var remoteView: RemoteView? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        handleIntent(intent)
        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                if (!vm.back()) finish()
            }
        })
        setContent {
            LanLinkTheme {
                val screen by vm.screen.collectAsState()
                immersive(screen == Screen.Remote)
                protectScreen(screen == Screen.Remote && !vm.app.settings.allowScreenshots)
                AppRoot(vm, this)
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIntent(intent)
    }

    private fun handleIntent(intent: Intent?) {
        val data = intent?.data ?: return
        // Handle a link once: the intent is redelivered to onCreate when the
        // process is recreated, which must not re-offer the pairing.
        intent.data = null
        setIntent(intent)
        if (data.scheme == "lanlink" && data.host == "pair") {
            // A link can come from any web page or app, so it only proposes
            // pairing; nothing connects until the user confirms on screen.
            // Onboarding is completed only if the user accepts (acceptOffer).
            vm.go(Screen.Pair())
            vm.offerPairFromLink(data.toString())
        }
    }

    /**
     * Keep the PC's screen out of screenshots, screen recordings, casting
     * and the Recents thumbnail while it is shown (FLAG_SECURE).
     */
    private fun protectScreen(on: Boolean) {
        if (on) window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        else window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
    }

    /** Full screen while controlling the PC; normal bars elsewhere. */
    private fun immersive(on: Boolean) {
        val c = WindowInsetsControllerCompat(window, window.decorView)
        WindowCompat.setDecorFitsSystemWindows(window, false)
        if (on) {
            c.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            c.hide(WindowInsetsCompat.Type.systemBars())
        } else {
            c.show(WindowInsetsCompat.Type.systemBars())
        }
    }

    fun keepScreenOn(on: Boolean) {
        if (on) window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        else window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
    }
}
