package domica.caprust.editor

import android.content.Context
import android.util.Log
import java.io.File

/**
 * The engine, running on the phone.
 *
 * The app does not talk to a CapRust on a desktop. It starts the
 * engine itself: `caprust-androidd`, which the package manager put in
 * the app's native library folder -- the only place an Android app may
 * run a binary from, since Android 10 stopped letting apps run files
 * out of their own data directory.
 *
 * The daemon prints one line when it is listening: the port and the
 * token. The token is not a secret from anybody: the daemon and the
 * app are the same device on loopback. It is there because the server
 * asks every call to carry one, and the server is not the one that
 * decides.
 */
class Daemon(private val ctx: Context) {

    companion object {
        private const val TAG = "CapRustDaemon"

        /** The daemon's binary, as the package manager extracted it. */
        const val LIB = "libcaprust-androidd.so"

        /**
         * The ffmpeg the engine renders with, in the same folder.
         *
         * The name ends in `.so` on purpose. The package manager is
         * sure about the ones that do -- it extracts them and sets the
         * exec bit -- and it makes no promise about the others.
         */
        const val FFMPEG = "libffmpeg.so"

        /** The ffprobe the engine probes with. */
        const val FFPROBE = "libffprobe.so"
    }

    private var process: Process? = null

    /** Where the engine keeps its files: the app's own directory. */
    private fun workDir(): File = File(ctx.filesDir, "work").apply { mkdirs() }

    /**
     * Start the engine and hand the port and the token over.
     *
     * The call is made on a thread of its own: the daemon's first
     * line only comes after it has bound its socket, and reading it
     * on the main thread would block the window.
     */
    fun start(then: (Boolean, String, Int, String) -> Unit) {
        stop()
        val native = File(ctx.applicationInfo.nativeLibraryDir)
        val exe = File(native, LIB)
        if (!exe.exists()) {
            then(false, "the engine is not in the app: ${exe.path}", 0, "")
            return
        }
        val work = workDir()
        val models = File(ctx.filesDir, "models").apply { mkdirs() }
        val project = File(work, "project.caprust")

        val process = try {
            ProcessBuilder(
                exe.absolutePath,
                "--ffmpeg", File(native, FFMPEG).absolutePath,
                "--ffprobe", File(native, FFPROBE).absolutePath,
                "--models", models.absolutePath,
                "--work", work.absolutePath,
                "--project", project.absolutePath,
            ).redirectErrorStream(false)
                // A ffmpeg that is not statically linked finds its
                // own libraries next to it, and the native library
                // folder is where they are.
                .apply { environment()["LD_LIBRARY_PATH"] = native.absolutePath }
                .start()
        } catch (e: Exception) {
            Log.e(TAG, "the engine did not start", e)
            then(false, e.message ?: "the engine did not start", 0, "")
            return
        }
        this.process = process

        // The engine's own complaints go to the log, not into the
        // window: they are for the developer, and the window has
        // better things to say.
        Thread {
            try {
                process.errorStream.bufferedReader().forEachLine {
                    Log.w(TAG, it)
                }
            } catch (_: Exception) {
                // The stream closes when the engine goes away.
            }
        }.apply { isDaemon = true }.start()

        Thread {
            val line = try {
                process.inputStream.bufferedReader().readLine()
            } catch (e: Exception) {
                null
            }
            if (line.isNullOrEmpty()) {
                then(false, "the engine did not say which port it is on", 0, "")
                return@Thread
            }
            val parts = line.trim().split(" ")
            val port = parts.getOrNull(0)?.toIntOrNull() ?: 0
            val token = parts.getOrNull(1) ?: ""
            then(port > 0, if (port > 0) "" else "the engine said '$line'", port, token)
        }.apply { isDaemon = true }.start()
    }

    /** Stop the engine. The app going away stops it anyway. */
    fun stop() {
        process?.destroy()
        process = null
    }
}
