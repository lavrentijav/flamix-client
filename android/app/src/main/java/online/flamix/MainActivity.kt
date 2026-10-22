package online.flamix

import android.app.NativeActivity
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.os.Bundle
import android.util.Log

/**
 * Hosts the Slint UI. Slint's Android backend requires NativeActivity or a subclass;
 * the Rust `android_main` (app/flamix-android) is loaded via the `android.app.lib_name`
 * meta-data in AndroidManifest.xml. VPN consent helper: [startVpn] (M3).
 *
 * M2 debug hook (debuggable builds only): start the activity with string extra `core`
 * = `start` or `stop`, e.g. `adb shell am start -n online.flamix.client/online.flamix.MainActivity --es core start`.
 */
class MainActivity : NativeActivity() {
    private val core by lazy { CoreService(this) }

    override fun onCreate(savedInstanceState: Bundle?) {
        FileLog.init(this)
        super.onCreate(savedInstanceState)
        FileLog.i(TAG, "MainActivity created")
        FlamixBridge.attach(this)
        handleDebugIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleDebugIntent(intent)
    }

    private fun handleDebugIntent(intent: Intent?) {
        if (applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE == 0) return
        when (intent?.getStringExtra("core")) {
            "start" -> Thread {
                try {
                    val ok = core.start(CoreService.installSampleConfig(this))
                    Log.i(TAG, "core start requested, started=$ok")
                } catch (e: Exception) {
                    Log.e(TAG, "core start failed", e)
                }
            }.start()
            "stop" -> Thread { core.stop(); Log.i(TAG, "core stopped") }.start()
        }
        // M3 debug: `--es vpn vpn_start` asks consent (first time) and starts the VpnService with the
        // sample config; `--es vpn vpn_stop` stops it. Runs on the main thread (consent dialog).
        when (intent?.getStringExtra("vpn")) {
            "vpn_start" -> startVpn(java.io.File(CoreService.installSampleConfig(this)).readText())
            "vpn_stop" -> stopVpn()
        }
    }

    private var pendingVpnConfig: String? = null

    /** Starts the system VPN with a sing-box config; asks for VPN consent first when needed. */
    fun startVpn(configJson: String) {
        FileLog.i(TAG, "startVpn requested")
        if (FlamixVpn.prepare(this)) FlamixVpn.start(this, configJson) else pendingVpnConfig = configJson
    }

    fun stopVpn() = FlamixVpn.stop(this)

    @Suppress("DEPRECATION")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != FlamixVpn.REQUEST_VPN) return
        val cfg = pendingVpnConfig.also { pendingVpnConfig = null }
        if (resultCode == RESULT_OK && cfg != null) FlamixVpn.start(this, cfg)
        else FileLog.w(TAG, "VPN consent denied/cancelled (result=$resultCode)")
    }

    override fun onDestroy() {
        FlamixBridge.detach(this)
        core.stop()
        super.onDestroy()
    }

    private companion object {
        const val TAG = "FlamixCore"
    }
}
