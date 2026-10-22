package online.flamix

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.VpnService

/**
 * Single entry point for the system-wide VPN mode.
 *
 * ```
 * FlamixVpn.prepare(activity, REQ)  // true -> consent already granted
 * FlamixVpn.start(context, configJson)
 * FlamixVpn.stop(context)
 * ```
 * [configJson] is the sing-box config from `flamix_core::config::android_tun::android_config` (proxy-only: a `mixed` inbound on
 * 127.0.0.1 without auth; [TunnelService] points tun2proxy at it).
 */
object FlamixVpn {
    const val REQUEST_VPN = 0x56504e // "VPN"

    /**
     * Asks for the system VPN consent dialog if needed. Returns true when consent is already
     * granted; otherwise starts the dialog and the result arrives in `onActivityResult`
     * with [requestCode] (RESULT_OK = granted).
     */
    @Suppress("DEPRECATION")
    fun prepare(activity: Activity, requestCode: Int = REQUEST_VPN): Boolean {
        val intent = VpnService.prepare(activity) ?: return true
        activity.startActivityForResult(intent, requestCode)
        return false
    }

    /** Starts the foreground VPN service. Call only after consent ([prepare] true / RESULT_OK). */
    fun start(context: Context, configJson: String) {
        val intent = Intent(context, TunnelService::class.java)
            .setAction(TunnelService.ACTION_START)
            .putExtra(TunnelService.EXTRA_CONFIG, configJson)
        context.startForegroundService(intent)
    }

    fun stop(context: Context) {
        context.startService(Intent(context, TunnelService::class.java).setAction(TunnelService.ACTION_STOP))
    }
}
