package domica.caprust.editor

import android.app.Activity
import android.content.Intent
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Bundle
import android.util.Base64
import android.util.Log
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import domica.caprust.remote.Client
import domica.caprust.remote.R
import org.json.JSONObject
import java.io.File

/**
 * CapRust on the phone.
 *
 * A real editor, not a remote control: the engine runs on the phone
 * (see [Daemon]) and this screen asks it questions. The wire format
 * is the one the companion already speaks, so the `Client` class is
 * the same one, unchanged.
 *
 * The first screen does the four things a Story needs: bring a file
 * in, put it on the timeline, look at a frame of it, render it.
 */
class EditorActivity : Activity() {

    companion object {
        private const val TAG = "CapRustEditor"
        private const val REQUEST_PICK = 41
    }

    private lateinit var client: Client
    private lateinit var daemon: Daemon
    private lateinit var status: TextView
    private lateinit var preview: ImageView
    private lateinit var timeline: LinearLayout
    private lateinit var exportProgress: TextView

    /** Where the picked file was copied to. The engine reads paths. */
    private var imported: File? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        client = Client(this)
        daemon = Daemon(this)
        build()
        startEngine()
    }

    override fun onDestroy() {
        previewTicker.removeCallbacks(previewRunnable)
        daemon.stop()
        client.disconnect()
        super.onDestroy()
    }

    // ---- the screen -------------------------------------------------

    private fun build() {
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(12), dp(12), dp(12), dp(12))
            setBackgroundColor(0xFF101014.toInt())
        }

        status = TextView(this).apply {
            setText(getString(R.string.editor_starting))
            setTextColor(0xFF9AA0A6.toInt())
            textSize = 13f
        }
        root.addView(status)

        preview = ImageView(this).apply {
            adjustViewBounds = true
            setBackgroundColor(0xFF1A1A20.toInt())
            contentDescription = getString(R.string.editor_preview)
        }
        root.addView(
            preview,
            LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
                1f,
            ),
        )

        val buttons = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        buttons.addView(
            button(R.string.editor_add_media) { pickMedia() },
            LinearLayout.LayoutParams(0, dp(44), 1f),
        )
        buttons.addView(
            button(R.string.editor_preview_button) { renderPreview() },
            LinearLayout.LayoutParams(0, dp(44), 1f),
        )
        buttons.addView(
            button(R.string.editor_export) { startExport() },
            LinearLayout.LayoutParams(0, dp(44), 1f),
        )
        root.addView(buttons)

        exportProgress = TextView(this).apply {
            setTextColor(0xFF8AB4F8.toInt())
            textSize = 13f
        }
        root.addView(exportProgress)

        val scroll = ScrollView(this)
        timeline = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(0, dp(8), 0, dp(8))
        }
        scroll.addView(timeline)
        root.addView(
            scroll,
            LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
                1f,
            ),
        )

        setContentView(root)
    }

    private fun button(label: Int, onClick: () -> Unit): Button =
        Button(this).apply {
            setText(label)
            setOnClickListener { onClick() }
        }

    private fun dp(v: Int): Int = (v * resources.displayMetrics.density).toInt()

    // ---- the engine -------------------------------------------------

    private fun startEngine() {
        daemon.start { ok, why, port, token ->
            runOnUiThread {
                if (!ok) {
                    status.text = getString(R.string.editor_engine_failed, why)
                    return@runOnUiThread
                }
                status.text = getString(R.string.editor_listening, port)
                client.setToken(token)
                client.connect("127.0.0.1", port) { connected, err ->
                    runOnUiThread {
                        if (!connected) {
                            status.text = getString(R.string.editor_engine_failed, err)
                            return@runOnUiThread
                        }
                        status.text = getString(R.string.editor_ready)
                        refresh()
                        previewTicker.post(previewRunnable)
                    }
                }
            }
        }
    }

    private val previewTicker = android.os.Handler(android.os.Looper.getMainLooper())
    private val previewRunnable = object : Runnable {
        override fun run() {
            renderPreview()
            previewTicker.postDelayed(this, 400)
        }
    }

    /** Ask the engine what is on the timeline. */
    private fun refresh() {
        client.call("list_clips") { ok, text ->
            runOnUiThread {
                timeline.removeAllViews()
                if (!ok) {
                    status.text = getString(R.string.editor_call_failed, text)
                    return@runOnUiThread
                }
                val clips = try {
                    org.json.JSONArray(text)
                } catch (e: Exception) {
                    org.json.JSONArray()
                }
                for (i in 0 until clips.length()) {
                    val clip = clips.optJSONObject(i) ?: continue
                    val label = clip.optString("name", clip.optString("path", "?"))
                    val tv = TextView(this).apply {
                        setText(labelStr + " │ " + (if (clip.optString("transition_in").isNotEmpty()) "→" else "") + (if (clip.optString("transition_out").isNotEmpty()) "←" else ""))
                        setTextColor(0xFFE8EAED.toInt())
                        setBackgroundColor(if (clip.optString("transition_in").isNotEmpty() || clip.optString("transition_out").isNotEmpty()) 0xFF8B3A3A.toInt() else 0xFF252530.toInt())
                        textSize = 13f
                        setPadding(dp(8), dp(4), dp(8), dp(4))
                    }
                    tv.setOnTouchListener { v, event ->
                        when (event.action) {
                            android.view.MotionEvent.ACTION_DOWN -> {
                                v.tag = event.rawX.toInt()
                                true
                            }
                            android.view.MotionEvent.ACTION_MOVE -> {
                                val prev = v.tag as? Int ?: event.rawX.toInt()
                                val dx = event.rawX.toInt() - prev
                                v.tag = event.rawX.toInt()
                                v.x += dx
                                true
                            }
                            else -> false
                        }
                    }
                    timeline.addView(tv)
                }
                if (clips.length() == 0) {
                    timeline.addView(
                        TextView(this).apply {
                            setText(getString(R.string.editor_empty_timeline))
                            setTextColor(0xFF9AA0A6.toInt())
                            textSize = 14f
                        },
                    )
                }
            }
        }
    }

    /** Pick a file, copy it where the engine can read it, and import it. */
    private fun pickMedia() {
        val i = Intent(Intent.ACTION_GET_CONTENT).apply { type = "*/*" }
        try {
            startActivityForResult(i, REQUEST_PICK)
        } catch (e: Exception) {
            Toast.makeText(this, R.string.editor_no_picker, Toast.LENGTH_LONG).show()
        }
    }

    @Deprecated("the platform asks for it this way")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != REQUEST_PICK || resultCode != RESULT_OK) return
        val uri: Uri = data?.data ?: return

        // The engine reads files by path, and a content URI is not a
        // path. The file is copied into the app's own directory, which
        // is also where the export goes.
        //
        // The name comes with it. The engine works out what a file is
        // from its name -- a file called "import-1759000000" is a
        // file of no known kind, and the import is refused.
        val mediaDir = File(filesDir, "media").apply { mkdirs() }
        val target = File(mediaDir, nameFor(uri))
        try {
            contentResolver.openInputStream(uri)?.use { input ->
                target.outputStream().use { output -> input.copyTo(output) }
            } ?: return
        } catch (e: Exception) {
            Log.e(TAG, "could not read the picked file", e)
            Toast.makeText(this, R.string.editor_import_failed, Toast.LENGTH_LONG).show()
            return
        }
        imported = target

        val args = JSONObject().put("path", target.absolutePath)
        // The kind goes with it. The engine works out what a file is
        // from its name, and a picker's name is not always a name: on
        // some devices the query for it comes back empty, and a file
        // called "import-1759000000.bin" is a file of no known kind.
        //
        // The type the picker reports is the first answer. If it does
        // not say anything useful, the file is asked itself: every
        // format has a few bytes at the front that say what it is.
        val kind = kindFor(uri) ?: sniffKind(target)
        kind?.let { args.put("kind", it) }
        client.call("add_media", args) { ok, text ->
            runOnUiThread {
                if (!ok) {
                    Toast.makeText(this, text, Toast.LENGTH_LONG).show()
                    return@runOnUiThread
                }
                Toast.makeText(this, R.string.editor_imported, Toast.LENGTH_SHORT).show()
                refresh()
            }
        }
    }

    /**
     * What the picker says the file is: "video", "audio" or "image".
     *
     * The type is more dependable than the name. A picker's name is
     * sometimes empty, and the engine needs to know what a file is
     * before it will take it.
     */
    private fun kindFor(uri: Uri): String? {
        val type = try {
            contentResolver.getType(uri)
        } catch (e: Exception) {
            null
        } ?: return null
        return when {
            type.startsWith("video/") -> "video"
            type.startsWith("audio/") -> "audio"
            type.startsWith("image/") -> "image"
            else -> null
        }
    }

    /**
     * A name for the copied file that keeps its kind.
     *
     * The extension is what the engine looks at, so it has to be
     * there: "clip.mp4" is a video, "clip" is nothing. The picker's
     * name is used when it has one, and the type fills in the
     * extension when it does not.
     */
    private fun nameFor(uri: Uri): String {
        val name = try {
            contentResolver
                .query(uri, null, null, null, null)
                ?.use { c ->
                    val i = c.getColumnIndex(android.provider.OpenableColumns.DISPLAY_NAME)
                    if (i >= 0 && c.moveToFirst()) c.getString(i) else null
                }
        } catch (e: Exception) {
            null
        }
        val clean = name
            ?.replace(Regex("[^A-Za-z0-9._-]"), "_")
            ?.takeIf { it.isNotBlank() }
        val dot = clean?.lastIndexOf('.') ?: -1
        val stem = if (dot > 0) clean!!.take(dot) else clean ?: "import-${System.currentTimeMillis()}"
        val ext = when {
            dot > 0 -> clean!!.substring(dot)
            else -> extensionFor(uri)
        }
        return "${System.currentTimeMillis()}-$stem$ext"
    }

    /**
     * What the file says it is, from its first bytes.
     *
     * Every format has a few bytes at the front that say what it is,
     * and the file is right about itself more often than a picker is.
     */
    private fun sniffKind(file: File): String? {
        val head = try {
            file.inputStream().use { input ->
                val buf = ByteArray(16)
                val read = input.read(buf)
                if (read <= 0) return null
                buf.copyOf(read)
            }
        } catch (e: Exception) {
            return null
        }
        fun starts(vararg magic: Int): Boolean {
            if (head.size < magic.size) return false
            return magic.withIndex().all { (i, b) -> (head[i].toInt() and 0xff) == b }
        }
        fun startsAt(offset: Int, vararg magic: Int): Boolean {
            if (head.size < offset + magic.size) return false
            return magic.withIndex().all { (i, b) -> (head[offset + i].toInt() and 0xff) == b }
        }
        return when {
            // MP4/MOV: 4-byte box size, then "ftyp" at offset 4.
            startsAt(4, 0x66, 0x74, 0x79, 0x70) -> "video"
            // WebM/MKV: EBML header at offset 0.
            starts(0x1a, 0x45, 0xdf, 0xa3) -> "video"
            // RIFF: avi or webm.
            starts(0x52, 0x49, 0x46, 0x46) -> "video"
            // Ogg container.
            starts(0x4f, 0x67, 0x67, 0x53) -> "audio"
            // MP3.
            starts(0x49, 0x44, 0x33) -> "audio"
            starts(0xff, 0xfb) || starts(0xff, 0xf3) -> "audio"
            // FLAC.
            starts(0x66, 0x4c, 0x61, 0x43) -> "audio"
            // Images.
            starts(0x89, 0x50, 0x4e, 0x47) -> "image"        // PNG
            starts(0xff, 0xd8) -> "image"                      // JPEG
            starts(0x47, 0x49, 0x46, 0x38) -> "image"        // GIF
            starts(0x42, 0x4d) -> "image"                     // BMP
            else -> null
        }
    }

    /** The extension the type implies, when the name has none. */
    private fun extensionFor(uri: Uri): String {
        val type = try {
            contentResolver.getType(uri)
        } catch (e: Exception) {
            null
        } ?: return ".bin"
        val sub = type.substringAfter('/')
        return if (sub.isNotBlank()) ".${sub.take(8)}" else ".bin"
    }

    /** Render one frame and show it. */
    private fun renderPreview() {
        status.text = getString(R.string.editor_rendering)
        client.call("preview_frame", JSONObject().put("width", 540)) { ok, text ->
            runOnUiThread {
                if (!ok || text.isNullOrEmpty()) {
                    status.text = getString(R.string.editor_call_failed, text)
                    return@runOnUiThread
                }
                try {
                    val bytes = Base64.decode(text, Base64.DEFAULT)
                    val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
                    if (bitmap == null) {
                        status.text = getString(R.string.editor_no_frame)
                        return@runOnUiThread
                    }
                    preview.setImageBitmap(bitmap)
                    status.text = getString(R.string.editor_ready)
                } catch (e: Exception) {
                    status.text = getString(R.string.editor_call_failed, e.message ?: "")
                }
            }
        }
    }

    /** Render the whole thing, and say how far it has got. */
    private fun startExport() {
        val out = File(filesDir, "export").apply { mkdirs() }
            .let { File(it, "story-${System.currentTimeMillis()}.mp4") }
        client.call("start_export", JSONObject().put("destination", out.absolutePath)) { ok, text ->
            runOnUiThread {
                if (!ok) {
                    exportProgress.text = getString(R.string.editor_call_failed, text)
                    return@runOnUiThread
                }
                exportProgress.text = getString(R.string.editor_exporting)
                pollExport(out)
            }
        }
    }

    /** Ask the engine how far the export has got, until it is done. */
    private fun pollExport(out: File) {
        client.call("export_status") { ok, text ->
            runOnUiThread {
                if (!ok) {
                    exportProgress.text = getString(R.string.editor_call_failed, text)
                    return@runOnUiThread
                }
                val progress = try {
                    JSONObject(text).optDouble("progress", -1.0)
                } catch (e: Exception) {
                    -1.0
                }
                if (progress >= 1.0) {
                    exportProgress.text = getString(R.string.editor_exported, out.name)
                    share(out)
                } else if (progress < 0.0) {
                    exportProgress.text = getString(R.string.editor_export_failed)
                } else {
                    exportProgress.text = getString(R.string.editor_exporting)
                    preview.postDelayed({ pollExport(out) }, 500)
                }
            }
        }
    }

    /** Hand the video to whatever the user wants to share it with. */
    private fun share(file: File) {
        if (!file.exists()) return
        try {
            val uri = putInTheGallery(file)
            if (uri == null) {
                Toast.makeText(this, file.absolutePath, Toast.LENGTH_LONG).show()
                return
            }
            val i = Intent(Intent.ACTION_SEND).apply {
                type = "video/mp4"
                putExtra(Intent.EXTRA_STREAM, uri)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            startActivity(Intent.createChooser(i, getString(R.string.editor_share)))
        } catch (e: Exception) {
            // No gallery, no share sheet: the file is on the device and
            // the path is in the log.
            Log.e(TAG, "could not share ${file.absolutePath}", e)
            Toast.makeText(this, file.absolutePath, Toast.LENGTH_LONG).show()
        }
    }

    /**
     * Put the video in the gallery and return where it went.
     *
     * A `file://` URI cannot be shared since Android 7 -- it throws
     * instead -- so the video goes into the gallery first and the
     * share sheet gets a URI the receiving app is allowed to read. The
     * gallery is also where a Story has to end up anyway.
     */
    private fun putInTheGallery(file: File): Uri? {
        val values = android.content.ContentValues().apply {
            put(android.provider.MediaStore.Video.Media.DISPLAY_NAME, file.name)
            put(android.provider.MediaStore.Video.Media.MIME_TYPE, "video/mp4")
            if (android.os.Build.VERSION.SDK_INT >= 29) {
                put(
                    android.provider.MediaStore.Video.Media.RELATIVE_PATH,
                    "Movies/CapRust",
                )
            }
        }
        val uri = contentResolver.insert(
            android.provider.MediaStore.Video.Media.EXTERNAL_CONTENT_URI, values,
        ) ?: return null
        contentResolver.openOutputStream(uri)?.use { out ->
            file.inputStream().use { it.copyTo(out) }
        }
        return uri
    }
}
