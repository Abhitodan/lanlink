package com.lanlink.app

import android.app.Application
import android.content.Context
import android.hardware.display.DisplayManager
import android.view.Display
import android.view.WindowManager
import com.lanlink.app.core.Discovery
import com.lanlink.app.core.HostRepository
import com.lanlink.app.core.IdentityStore
import com.lanlink.app.core.Settings

/** Process-wide services. Small, explicit, no DI framework needed. */
class LanLinkApp : Application() {
    lateinit var identity: IdentityStore
        private set
    lateinit var hosts: HostRepository
        private set
    lateinit var settings: Settings
        private set
    lateinit var discovery: Discovery
        private set

    override fun onCreate() {
        super.onCreate()
        identity = IdentityStore(this)
        hosts = HostRepository(this)
        settings = Settings(this)
        discovery = Discovery(this)
    }

    /** Physical screen size in pixels, long edge first. */
    fun displayMetrics(): Pair<Int, Int> {
        val (w, h) = if (android.os.Build.VERSION.SDK_INT >= 30) {
            val b = (getSystemService(Context.WINDOW_SERVICE) as WindowManager).maximumWindowMetrics.bounds
            b.width() to b.height()
        } else {
            val dm = getSystemService(Context.DISPLAY_SERVICE) as DisplayManager
            val m = android.util.DisplayMetrics()
            @Suppress("DEPRECATION")
            dm.getDisplay(Display.DEFAULT_DISPLAY)?.getRealMetrics(m)
            m.widthPixels to m.heightPixels
        }
        return maxOf(w, h) to minOf(w, h)
    }

    /** The display's highest supported refresh rate (not an assumption of 120 Hz). */
    fun refreshRateHz(): Float {
        val dm = getSystemService(Context.DISPLAY_SERVICE) as DisplayManager
        val d = dm.getDisplay(Display.DEFAULT_DISPLAY) ?: return 60f
        return d.supportedModes.maxOfOrNull { it.refreshRate } ?: d.refreshRate
    }
}
