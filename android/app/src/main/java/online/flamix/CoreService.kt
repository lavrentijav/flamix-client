package online.flamix

import android.content.Context
import java.io.File
import java.util.concurrent.TimeUnit

/**
 * Runs the proxy core (sing-box, shipped as libsingbox.so in nativeLibraryDir) as a child process.
 * Plain class, no UI. Output of the process is kept in a ring buffer ([logTail]).
 */
class CoreService(
    private val binary: File,
    private val workDir: File,
    private val maxLogLines: Int = 200,
) {
    constructor(context: Context) : this(
        File(context.applicationInfo.nativeLibraryDir, "libsingbox.so"),
        context.filesDir,
    )

    private val lock = Any()
    private var process: Process? = null
    private val ring = ArrayDeque<String>()

    val isRunning: Boolean
        get() = synchronized(lock) { process?.isAlive == true }

    /** Starts the core with the given config file. Returns false if already running. */
    fun start(configPath: String): Boolean = synchronized(lock) {
        if (process?.isAlive == true) return false
        ring.clear()
        val p = ProcessBuilder(binary.path, "run", "-c", configPath)
            .directory(workDir)
            .redirectErrorStream(true)
            .start()
        process = p
        Thread({ pump(p) }, "core-log").apply { isDaemon = true }.start()
        true
    }

    /** destroy(), then destroyForcibly() if the process is still alive after 3 seconds. */
    fun stop() {
        val p = synchronized(lock) { process } ?: return
        p.destroy()
        if (!p.waitFor(3, TimeUnit.SECONDS)) {
            p.destroyForcibly()
            p.waitFor(3, TimeUnit.SECONDS)
        }
        synchronized(lock) { if (process === p) process = null }
    }

    /** Last lines of core output (oldest first). */
    fun logTail(): List<String> = synchronized(lock) { ring.toList() }

    private fun pump(p: Process) {
        try {
            p.inputStream.bufferedReader().forEachLine { line ->
                synchronized(lock) {
                    ring.addLast(line)
                    while (ring.size > maxLogLines) ring.removeFirst()
                }
            }
        } catch (_: java.io.IOException) {
            // stream closed on stop
        }
    }

    companion object {
        const val SAMPLE_ASSET = "singbox-sample.json"

        /** Copies the bundled sample config into filesDir and returns its path. */
        fun installSampleConfig(context: Context): String {
            val out = File(context.filesDir, SAMPLE_ASSET)
            context.assets.open(SAMPLE_ASSET).use { i -> out.outputStream().use { i.copyTo(it) } }
            return out.path
        }
    }
}
