package online.flamix

import android.app.NativeActivity

/**
 * Hosts the Slint UI. Slint's Android backend requires NativeActivity or a subclass;
 * the Rust `android_main` (app/flamix-android) is loaded via the `android.app.lib_name`
 * meta-data in AndroidManifest.xml. VPN code will be added here in a later milestone.
 */
class MainActivity : NativeActivity()
