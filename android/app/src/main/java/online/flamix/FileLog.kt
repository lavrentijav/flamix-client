package online.flamix

import android.content.Context
import android.util.Log
import java.io.File

/**
 * Appends to `<files dir>/flamix.log`, the same file the Rust side writes (applog.rs).
 * Format `unixSeconds LEVEL tag: message`; rotates at 200 KB to `flamix.log.1`.
 */
object FileLog {
    private const val MAX_BYTES = 200 * 1024L
    @Volatile private var dir: File? = null

    fun init(context: Context) {
        dir = context.filesDir
        val previous = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { t, e ->
            e("uncaught", "thread ${t.name}", e)
            previous?.uncaughtException(t, e)
        }
    }

    fun i(tag: String, msg: String) = write("I", tag, msg).also { Log.i(tag, msg) }

    fun w(tag: String, msg: String) = write("W", tag, msg).also { Log.w(tag, msg) }

    fun e(tag: String, msg: String, t: Throwable? = null) {
        val text = if (t == null) msg else "$msg: ${Log.getStackTraceString(t)}"
        Log.e(tag, text)
        write("E", tag, text)
    }

    @Synchronized
    private fun write(level: String, tag: String, msg: String) {
        val d = dir ?: return
        try {
            val f = File(d, "flamix.log")
            if (f.length() > MAX_BYTES) f.renameTo(File(d, "flamix.log.1"))
            f.appendText("${System.currentTimeMillis() / 1000} $level $tag: $msg\n")
        } catch (_: Exception) {
        }
    }
}
