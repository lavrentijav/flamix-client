package online.flamix

/**
 * Entry point the Rust UI calls through JNI (app/flamix-android/src/bridge.rs).
 *
 * Static methods only. Rust looks this class up through the activity's class loader and calls
 * [connect], [disconnect] and [isRunning]. VPN consent stays here: [connect] goes through
 * [MainActivity.startVpn], which asks `VpnService.prepare` before starting the tunnel.
 */
object FlamixBridge {
    @Volatile
    private var activity: MainActivity? = null

    /** Called from [MainActivity.onCreate]. */
    fun attach(activity: MainActivity) {
        this.activity = activity
    }

    /** Called from [MainActivity.onDestroy]. */
    fun detach(activity: MainActivity) {
        if (this.activity === activity) this.activity = null
    }

    /** Starts the VPN with a sing-box config (consent dialog first if needed). False = no activity. */
    @JvmStatic
    fun connect(configJson: String): Boolean {
        val act = activity ?: return false
        act.runOnUiThread { act.startVpn(configJson) }
        return true
    }

    /** Stops the VPN service. False = no activity. */
    @JvmStatic
    fun disconnect(): Boolean {
        val act = activity ?: return false
        act.runOnUiThread { act.stopVpn() }
        return true
    }

    /** Whether the native tunnel is forwarding packets. */
    @JvmStatic
    fun isRunning(): Boolean = NativeTunnel.isRunning()
}
