package online.flamix

import android.content.Context
import android.content.Intent
import android.net.Uri

/**
 * Opens provider links (support, site, banner button) in the system app for them.
 *
 * Called from Rust through JNI (app/flamix-android/src/external.rs) with the activity as the
 * context. Returns false when no app can open the link.
 */
object FlamixLinks {
    @JvmStatic
    fun open(context: Context, url: String): Boolean {
        return try {
            val intent = Intent(Intent.ACTION_VIEW, Uri.parse(url))
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            context.startActivity(intent)
            true
        } catch (e: Exception) {
            false
        }
    }
}
