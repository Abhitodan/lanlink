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
        val (w, h) = runCatching {
            if (android.os.Build.VERSION.SDK_INT >= 30) {
                val b = (getSystemService(Context.WINDOW_SERVICE) as WindowManager).maximumWindowMetrics.bounds
                b.width() to b.height()
            } else {
                val dm = getSystemService(Context.DISPLAY_SERVICE) as DisplayManager
                val m = android.util.DisplayMetrics()
                @Suppress("DEPRECATION")
                dm.getDisplay(Display.DEFAULT_DISPLAY)?.getRealMetrics(m)
                m.widthPixels to m.heightPixels
            }
        }.getOrElse { 0 to 0 }.let { (rw, rh) ->
            if (rw > 0 && rh > 0) rw to rh
            else resources.displayMetrics.widthPixels to resources.displayMetrics.heightPixels
        }
        return maxOf(w, h).coerceAtLeast(640) to minOf(w, h).coerceAtLeast(480)
    }

    /** The display's highest supported refresh rate (not an assumption of 120 Hz). */
    fun refreshRateHz(): Float {
        val rate = runCatching {
            val dm = getSystemService(Context.DISPLAY_SERVICE) as DisplayManager
            val d = dm.getDisplay(Display.DEFAULT_DISPLAY) ?: return 60f
            d.supportedModes.maxOfOrNull { it.refreshRate } ?: d.refreshRate
        }.getOrDefault(60f)
        return if (rate.isFinite() && rate > 0f) rate.coerceIn(30f, 240f) else 60f
    }
}
