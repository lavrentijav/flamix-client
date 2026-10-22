package online.flamix

/**
 * JNI bridge to app/flamix-android/src/tunnel.rs: tun2proxy reads the VpnService fd in-process
 * and forwards it to the core's local SOCKS5 inbound.
 */
object NativeTunnel {
    init {
        System.loadLibrary("flamix_android")
    }

    /** Starts forwarding on [fd] (borrowed, not closed) to socks5://127.0.0.1:[socksPort]. */
    @JvmStatic external fun start(fd: Int, socksPort: Int, mtu: Int): Boolean

    @JvmStatic external fun stop()

    @JvmStatic external fun isRunning(): Boolean

    @JvmStatic external fun lastError(): String
}
