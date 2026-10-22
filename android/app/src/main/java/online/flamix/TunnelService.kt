package online.flamix

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.VpnService
import android.os.Build
import android.os.ParcelFileDescriptor
import android.util.Log
import org.json.JSONObject
import java.io.File
import java.net.InetSocketAddress
import java.net.Socket

/**
 * System-wide VPN: owns the TUN interface and runs the sing-box core on it.
 *
 * Data path: TUN fd -> tun2proxy (in-process, JNI) -> sing-box mixed inbound on 127.0.0.1 -> proxy.
 * The app itself is excluded from the tunnel (addDisallowedApplication); the core child process
 * runs with the app's uid, so its outbound sockets use the underlying network, no protect() needed.
 */
class TunnelService : VpnService() {
    private var tun: ParcelFileDescriptor? = null
    private val core by lazy { CoreService(this) }

    override fun onCreate() {
        super.onCreate()
        FileLog.init(this)
        FileLog.i(TAG, "service created")
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        FileLog.i(TAG, "onStartCommand action=${intent?.action}")
        when (intent?.action) {
            ACTION_STOP -> {
                FileLog.i(TAG, "stop requested by app")
                shutdown()
                stopSelf()
            }
            ACTION_START -> {
                // startForegroundService() requires startForeground() within a few seconds.
                startForegroundCompat()
                val config = intent.getStringExtra(EXTRA_CONFIG)
                if (config == null || !startTunnel(config)) {
                    FileLog.e(TAG, "tunnel start failed (config=${config != null}); stopping service")
                    shutdown()
                    stopSelf()
                }
            }
        }
        return START_NOT_STICKY
    }

    private fun startTunnel(configJson: String): Boolean {
        shutdown()
        val pfd = try {
            Builder()
                .setSession("Flamix")
                .addAddress("172.19.0.1", 30)
                .addRoute("0.0.0.0", 0)
                .setMtu(1500)
                .addDnsServer("1.1.1.1")
                .addDisallowedApplication(packageName)
                .establish()
        } catch (e: Exception) {
            FileLog.e(TAG, "establish failed", e)
            null
        } ?: return false
        tun = pfd
        // The fd stays owned by this process; tun2proxy only borrows it (see tunnel.rs).
        val fd = pfd.fd
        return try {
            val cfg = JSONObject(configJson)
            val port = socksPort(cfg)
            val cfgFile = File(filesDir, "singbox-vpn.json").apply { writeText(cfg.toString()) }
            if (!core.start(cfgFile.path)) {
                FileLog.e(TAG, "core already running")
                return false
            }
            if (!waitForPort(port)) {
                FileLog.e(TAG, "core did not open 127.0.0.1:$port (alive=${core.isRunning}); core output:\n${core.logTail().takeLast(30).joinToString("\n")}")
                return false
            }
            val ok = NativeTunnel.start(fd, port, MTU)
            if (!ok) FileLog.e(TAG, "NativeTunnel.start false: ${NativeTunnel.lastError()}") else FileLog.i(TAG, "tunnel started fd=$fd port=$port")
            ok
        } catch (e: Exception) {
            FileLog.e(TAG, "tunnel start failed", e)
            false
        }
    }

    /** Port of the first `mixed`/`socks` inbound of the (proxy-only) sing-box config. */
    private fun socksPort(cfg: JSONObject): Int {
        val inbounds = cfg.getJSONArray("inbounds")
        for (i in 0 until inbounds.length()) {
            val ib = inbounds.getJSONObject(i)
            if (ib.optString("type") in listOf("mixed", "socks")) return ib.getInt("listen_port")
        }
        throw IllegalArgumentException("config has no mixed/socks inbound")
    }

    private fun waitForPort(port: Int): Boolean {
        val deadline = System.currentTimeMillis() + 20000
        while (System.currentTimeMillis() < deadline) {
            if (!core.isRunning) return false
            try {
                Socket().use { it.connect(InetSocketAddress("127.0.0.1", port), 200) }
                return true
            } catch (_: Exception) {
                Thread.sleep(100)
            }
        }
        return false
    }

    private fun shutdown() {
        NativeTunnel.stop()
        core.stop()
        try { tun?.close() } catch (_: Exception) {}
        tun = null
        stopForeground(STOP_FOREGROUND_REMOVE)
    }

    override fun onRevoke() {
        FileLog.w(TAG, "onRevoke (VPN revoked by system/user)")
        // User turned the VPN off in system settings.
        shutdown()
        stopSelf()
    }

    override fun onDestroy() {
        FileLog.w(TAG, "service destroyed; core output:\n${core.logTail().takeLast(15).joinToString("\n")}")
        shutdown()
        super.onDestroy()
    }

    private fun startForegroundCompat() {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL, "VPN", NotificationManager.IMPORTANCE_LOW),
        )
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n: Notification = Notification.Builder(this, CHANNEL)
            .setContentTitle("Flamix")
            .setContentText("VPN is active")
            .setSmallIcon(android.R.drawable.ic_lock_lock)
            .setOngoing(true)
            .setContentIntent(open)
            .build()
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(NOTIFICATION_ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        } else {
            startForeground(NOTIFICATION_ID, n)
        }
    }

    companion object {
        const val ACTION_START = "online.flamix.vpn.START"
        const val ACTION_STOP = "online.flamix.vpn.STOP"
        const val EXTRA_CONFIG = "config"
        private const val CHANNEL = "vpn"
        private const val NOTIFICATION_ID = 1
        private const val MTU = 1500
        private const val TAG = "FlamixVpn"
    }
}
