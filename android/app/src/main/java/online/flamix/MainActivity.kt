package online.flamix

import android.app.NativeActivity
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.os.Bundle
import android.util.Log

/**
 * Hosts the Slint UI. Slint's Android backend requires NativeActivity or a subclass;
 * the Rust `android_main` (app/flamix-android) is loaded via the `android.app.lib_name`
 * meta-data in AndroidManifest.xml. VPN code will be added here in a later milestone.
 *
 * M2 debug hook (debuggable builds only): start the activity with string extra `core`
 * = `start` or `stop`, e.g. `adb shell am start -n online.flamix.client/online.flamix.MainActivity --es core start`.
 */
class MainActivity : NativeActivity() {
    private val core by lazy { CoreService(this) }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
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
    }

    override fun onDestroy() {
        core.stop()
        super.onDestroy()
    }

    private companion object {
        const val TAG = "FlamixCore"
    }
}
