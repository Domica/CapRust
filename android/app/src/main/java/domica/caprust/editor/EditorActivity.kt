package domica.caprust.editor

import android.app.Activity
import android.content.Intent
import android.graphics.BitmapFactory
import android.graphics.Typeface
import android.net.Uri
import android.os.Bundle
import android.util.Base64
import android.util.Log
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
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
 * CapRust on the phone — standalone editor.
 *
 * The engine runs on the phone (see [Daemon]) and this activity drives it.
 * On first launch the user sees a Home screen with "New project" and
 * "Recent projects". Creating or opening a project switches to the Editor
 * screen: preview, timeline, add media, export.
 *
 * The wire format the engine speaks is the same JSON-RPC the desktop MCP
 * server uses, so the [Client] class is shared with the remote companion.
 */
class EditorActivity : Activity() {

    companion object {
        private const val TAG = "CapRustEditor"
        private const val REQUEST_PICK = 41
        private const val REQUEST_PICK_FOLDER = 42
    }

    /** Which screen is showing. */
    private enum class Screen { Home, New, Editor }

    private var current = Screen.Home

    private lateinit var client: Client
    private lateinit var daemon: Daemon
    private var status: TextView? = null

    /** Where the picked file was copied to. The engine reads paths. */
    private var imported: File? = null

    /** New-project form fields. */
    private var newName: EditText? = null
    private var newLocation: EditText? = null
    private var newResolution: Int = 1080
    private var newFpsLabel: String = "30 fps"

    /** Folder picked via the system folder picker, if any. */
    private var pickedFolderUri: Uri? = null

    /** Preview + timeline views (built once, reused). */
    private lateinit var preview: ImageView
    private lateinit var timeline: LinearLayout
    private lateinit var exportProgress: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        client = Client(this)
        daemon = Daemon(this)
        startEngine()
        show(current)
    }

    override fun onDestroy() {
        previewTicker.removeCallbacks(previewRunnable)
        daemon.stop()
        client.disconnect()
        super.onDestroy()
    }

    // ---- navigation -----------------------------------------------------

    /** Rebuild the content view for [screen]. */
    private fun show(screen: Screen) {
        current = screen
        setContentView(
            when (screen) {
                Screen.Home -> buildHome()
                Screen.New -> buildNewProject()
                Screen.Editor -> buildEditor()
            },
        )
        if (screen == Screen.Editor) {
            refresh()
            previewTicker.post(previewRunnable)
        }
    }

    private fun backToHome() {
        show(Screen.Home)
    }

    // ---- home screen ----------------------------------------------------

    /** Root menu: New project or Recent projects. */
    private fun buildHome(): View {
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(24), dp(24), dp(24), dp(24))
            setBackgroundColor(0xFF101014.toInt())
            gravity = Gravity.CENTER
        }

        val title = TextView(this).apply {
            text = getString(R.string.editor_home_title)
            textSize = 32f
            setTypeface(Typeface.create(Typeface.DEFAULT, Typeface.BOLD))
            setTextColor(0xFFE8EAED.toInt())
        }
        root.addView(title)

        root.addView(
            TextView(this).apply {
                text = getString(R.string.editor_ready)
                textSize = 14f
                setTextColor(0xFF9AA0A6.toInt())
                setPadding(0, dp(8), 0, dp(24))
            },
        )

        val newBtn = Button(this).apply {
            text = getString(R.string.editor_btn_new_project)
            textSize = 18f
            setAllCaps(false)
            setPadding(0, dp(16), 0, dp(16))
            setOnClickListener { show(Screen.New) }
        }
        root.addView(newBtn, LinearLayout.LayoutParams(dp(220), dp(52)))

        root.addView(
            TextView(this).apply {
                setPadding(0, dp(12), 0, dp(12))
            },
        )

        val recentBtn = Button(this).apply {
            text = getString(R.string.editor_btn_recent)
            textSize = 18f
            setAllCaps(false)
            setPadding(0, dp(16), 0, dp(16))
            setOnClickListener { showRecent() }
        }
        root.addView(recentBtn, LinearLayout.LayoutParams(dp(220), dp(52)))

        return root
    }

    /** Show recent projects — inline on the Home screen for now. */
    private fun showRecent() {
        val prefs = getSharedPreferences("caprust_editor", Activity.MODE_PRIVATE)
        val recent = prefs.getString("recent_projects", "[]") ?: "[]"
        val arr = try {
            org.json.JSONArray(recent)
        } catch (_: Exception) {
            org.json.JSONArray()
        }

        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(20), dp(16), dp(20), dp(16))
            setBackgroundColor(0xFF101014.toInt())
        }

        root.addView(
            TextView(this).apply {
                text = getString(R.string.editor_recent_title)
                textSize = 22f
                setTypeface(Typeface.create(Typeface.DEFAULT, Typeface.BOLD))
                setTextColor(0xFFE8EAED.toInt())
                setPadding(0, dp(4), 0, dp(16))
            },
        )

        if (arr.length() == 0) {
            root.addView(
                TextView(this).apply {
                    text = getString(R.string.editor_no_recent)
                    textSize = 15f
                    setTextColor(0xFF9AA0A6.toInt())
                    setPadding(0, dp(8), 0, dp(8))
                },
            )
        } else {
            for (i in 0 until arr.length()) {
                val entry = arr.optJSONObject(i) ?: continue
                val name = entry.optString("name", "?")
                val path = entry.optString("path", "")
                val res = entry.optInt("resolution", 1080)
                val fps = entry.optString("fps", "30 fps")

                val row = LinearLayout(this).apply {
                    orientation = LinearLayout.VERTICAL
                }
                row.addView(
                    Button(this).apply {
                        text = "$name   ($res px, $fps)"
                        textSize = 16f
                        setAllCaps(false)
                        setPadding(0, dp(12), 0, dp(12))
                        setOnClickListener {
                            openRecent(name, path)
                        }
                    },
                    LinearLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT, dp(52),
                    ),
                )
                root.addView(row)
                root.addView(
                    TextView(this).apply {
                        text = path
                        textSize = 11f
                        setTextColor(0xFF6A6F76.toInt())
                        setPadding(0, dp(2), 0, dp(10))
                    },
                )
            }
        }

        root.addView(
            TextView(this).apply {
                setPadding(0, dp(16), 0, dp(16))
            },
        )

        root.addView(
            Button(this).apply {
                text = getString(R.string.editor_back)
                textSize = 16f
                setAllCaps(false)
                setPadding(0, dp(16), 0, dp(16))
                setOnClickListener { show(Screen.Home) }
            },
            LinearLayout.LayoutParams(dp(160), dp(52)),
        )

        setContentView(root)
        current = Screen.Home // keep logical state on Home
    }

    /** Open a recent project and switch to the editor. */
    private fun openRecent(name: String, path: String) {
        val prefs = getSharedPreferences("caprust_editor", Activity.MODE_PRIVATE)
        prefs.edit()
            .putString("project_name", name)
            .putString("project_path", path)
            .apply()
        status?.text = "Opened: $name"
        show(Screen.Editor)
    }

    // ---- new project screen ---------------------------------------------

    /** Create-project form: name, location, resolution, FPS. */
    private fun buildNewProject(): View {
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(20), dp(16), dp(20), dp(16))
            setBackgroundColor(0xFF101014.toInt())
        }

        root.addView(
            TextView(this).apply {
                text = getString(R.string.editor_new_title)
                textSize = 22f
                setTypeface(Typeface.create(Typeface.DEFAULT, Typeface.BOLD))
                setTextColor(0xFFE8EAED.toInt())
                setPadding(0, dp(4), 0, dp(16))
            },
        )

        // Name
        root.addView(
            TextView(this).apply {
                text = getString(R.string.editor_field_name)
                textSize = 14f
                setTextColor(0xFF9AA0A6.toInt())
                setPadding(0, dp(6), 0, dp(2))
            },
        )
        newName = EditText(this).apply {
            hint = "My project"
            setTextColor(0xFFE8EAED.toInt())
            setHintTextColor(0xFF6A6F76.toInt())
        }
        root.addView(
            newName,
            LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, dp(48),
            ),
        )

        root.addView(
            TextView(this).apply {
                setPadding(0, dp(10), 0, dp(2))
            },
        )

        // Location
        root.addView(
            TextView(this).apply {
                text = getString(R.string.editor_field_location)
                textSize = 14f
                setTextColor(0xFF9AA0A6.toInt())
                setPadding(0, dp(6), 0, dp(2))
            },
        )
        val locRow = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        newLocation = EditText(this).apply {
            hint = getExternalFilesDir(null)?.absolutePath ?: ""
            setTextColor(0xFFE8EAED.toInt())
            setHintTextColor(0xFF6A6F76.toInt())
        }
        locRow.addView(
            newLocation,
            LinearLayout.LayoutParams(0, dp(48), 1f),
        )
        locRow.addView(
            Button(this).apply {
                text = getString(R.string.editor_btn_browse)
                textSize = 14f
                setAllCaps(false)
                setPadding(0, dp(10), 0, dp(10))
                setOnClickListener {
                    val i = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
                        addCategory(Intent.CATEGORY_OPENABLE)
                    }
                    try {
                        startActivityForResult(i, REQUEST_PICK_FOLDER)
                    } catch (e: Exception) {
                        Toast.makeText(this, R.string.editor_no_picker, Toast.LENGTH_LONG).show()
                    }
                }
            },
            LinearLayout.LayoutParams(dp(96), dp(48)),
        )
        root.addView(locRow)

        root.addView(
            TextView(this).apply {
                setPadding(0, dp(10), 0, dp(2))
            },
        )

        // Resolution
        root.addView(
            TextView(this).apply {
                text = getString(R.string.editor_field_resolution)
                textSize = 14f
                setTextColor(0xFF9AA0A6.toInt())
                setPadding(0, dp(6), 0, dp(2))
            },
        )
        val resRow = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        resRow.addView(
            TextView(this).apply {
                text = "$newResolution px"
                textSize = 14f
                setTextColor(0xFFE8EAED.toInt())
            },
            LinearLayout.LayoutParams(dp(80), dp(48)),
        )
        resRow.addView(
            Button(this).apply {
                text = "-"
                textSize = 20f
                setAllCaps(false)
                setPadding(0, dp(8), 0, dp(8))
                setOnClickListener {
                    if (newResolution > 480) {
                        newResolution -= 120
                        (resRow.getChildAt(0) as TextView).text = "$newResolution px"
                    }
                }
            },
            LinearLayout.LayoutParams(dp(44), dp(44)),
        )
        resRow.addView(
            Button(this).apply {
                text = "+"
                textSize = 20f
                setAllCaps(false)
                setPadding(0, dp(8), 0, dp(8))
                setOnClickListener {
                    if (newResolution < 2160) {
                        newResolution += 120
                        (resRow.getChildAt(0) as TextView).text = "$newResolution px"
                    }
                }
            },
            LinearLayout.LayoutParams(dp(44), dp(44)),
        )
        root.addView(resRow)

        root.addView(
            TextView(this).apply {
                setPadding(0, dp(10), 0, dp(2))
            },
        )

        // FPS
        root.addView(
            TextView(this).apply {
                text = getString(R.string.editor_field_fps)
                textSize = 14f
                setTextColor(0xFF9AA0A6.toInt())
                setPadding(0, dp(6), 0, dp(2))
            },
        )
        val fpsRow = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        fpsRow.addView(
            TextView(this).apply {
                text = newFpsLabel
                textSize = 14f
                setTextColor(0xFFE8EAED.toInt())
            },
            LinearLayout.LayoutParams(dp(80), dp(48)),
        )
        val fpsOptions = listOf("24 fps", "30 fps", "60 fps")
        var fpsIdx = fpsOptions.indexOf(newFpsLabel)
        fpsRow.addView(
            Button(this).apply {
                text = "<"
                textSize = 20f
                setAllCaps(false)
                setPadding(0, dp(8), 0, dp(8))
                setOnClickListener {
                    fpsIdx = (fpsIdx - 1 + fpsOptions.size) % fpsOptions.size
                    newFpsLabel = fpsOptions[fpsIdx]
                    (fpsRow.getChildAt(0) as TextView).text = newFpsLabel
                }
            },
            LinearLayout.LayoutParams(dp(44), dp(44)),
        )
        fpsRow.addView(
            Button(this).apply {
                text = ">"
                textSize = 20f
                setAllCaps(false)
                setPadding(0, dp(8), 0, dp(8))
                setOnClickListener {
                    fpsIdx = (fpsIdx + 1) % fpsOptions.size
                    newFpsLabel = fpsOptions[fpsIdx]
                    (fpsRow.getChildAt(0) as TextView).text = newFpsLabel
                }
            },
            LinearLayout.LayoutParams(dp(44), dp(44)),
        )
        root.addView(fpsRow)

        root.addView(
            TextView(this).apply {
                setPadding(0, dp(16), 0, dp(16))
            },
        )

        // Buttons
        val btnRow = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        btnRow.addView(
            Button(this).apply {
                text = getString(R.string.editor_btn_create)
                textSize = 16f
                setAllCaps(false)
                setPadding(0, dp(16), 0, dp(16))
                setOnClickListener { doCreateProject() }
            },
            LinearLayout.LayoutParams(0, dp(52), 1f),
        )
        btnRow.addView(
            TextView(this).apply {
                setPadding(0, dp(8), 0, dp(8))
            },
        )
        btnRow.addView(
            Button(this).apply {
                text = getString(R.string.editor_btn_cancel)
                textSize = 16f
                setAllCaps(false)
                setPadding(0, dp(16), 0, dp(16))
                setOnClickListener { show(Screen.Home) }
            },
            LinearLayout.LayoutParams(0, dp(52), 1f),
        )
        root.addView(btnRow)

        return root
    }

    /** Validate and create the project, then switch to the editor. */
    private fun doCreateProject() {
        val name = newName?.text?.toString()?.trim() ?: ""
        if (name.isEmpty()) {
            Toast.makeText(this, R.string.editor_btn_create_hint, Toast.LENGTH_LONG).show()
            return
        }

        // Use the picked folder if one was chosen, otherwise the text field.
        val location = pickedFolderUri?.lastPathSegment
            ?: newLocation?.text?.toString()?.trim() ?: ""
        if (location.isEmpty()) {
            Toast.makeText(this, R.string.editor_btn_create_hint, Toast.LENGTH_LONG).show()
            return
        }

        val projectFolder = File(location, name).apply { mkdirs() }
        val prefs = getSharedPreferences("caprust_editor", Activity.MODE_PRIVATE)
        prefs.edit()
            .putString("project_name", name)
            .putString("project_path", projectFolder.absolutePath)
            .putInt("project_resolution", newResolution)
            .putString("project_fps", newFpsLabel)
            .putString("project_folder_uri", pickedFolderUri?.toString() ?: "")
            .apply()

        // Add to recent projects
        addToRecent(name, projectFolder.absolutePath)

        status?.text = "Created: $name"
        show(Screen.Editor)
    }

    /** Remember this project under "recent". */
    private fun addToRecent(name: String, path: String) {
        val prefs = getSharedPreferences("caprust_editor", Activity.MODE_PRIVATE)
        val existing = prefs.getString("recent_projects", "[]") ?: "[]"
        val arr = try {
            org.json.JSONArray(existing)
        } catch (_: Exception) {
            org.json.JSONArray()
        }
        // Remove duplicate if present
        for (i in 0 until arr.length()) {
            val o = arr.optJSONObject(i)
            if (o != null && o.optString("path") == path) {
                arr.remove(i)
                break
            }
        }
        val entry = org.json.JSONObject()
            .put("name", name)
            .put("path", path)
            .put("resolution", newResolution)
            .put("fps", newFpsLabel)
        arr.put(entry)
        // Keep only the last 10
        while (arr.length() > 10) {
            arr.remove(0)
        }
        prefs.edit().putString("recent_projects", arr.toString()).apply()
    }

    // ---- editor screen --------------------------------------------------

    /** The main editor: preview, timeline, add media, export. */
    private fun buildEditor(): View {
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(12), dp(12), dp(12), dp(12))
            setBackgroundColor(0xFF101014.toInt())
        }

        val header = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(0, 0, 0, dp(8))
            gravity = Gravity.CENTER_VERTICAL
        }
        header.addView(
            Button(this).apply {
                text = getString(R.string.editor_back)
                textSize = 14f
                setAllCaps(false)
                setPadding(0, dp(10), 0, dp(10))
                setOnClickListener { backToHome() }
            },
        )
        header.addView(
            TextView(this).apply {
                text = projectTitle()
                textSize = 17f
                setTypeface(Typeface.create(Typeface.DEFAULT, Typeface.BOLD))
                setTextColor(0xFFE8EAED.toInt())
                setPadding(dp(8), 0, 0, 0)
            },
        )
        root.addView(header)

        status = TextView(this).apply {
            text = getString(R.string.editor_ready)
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

        return root
    }

    /** Project name for the header, from preferences. */
    private fun projectTitle(): String {
        val prefs = getSharedPreferences("caprust_editor", Activity.MODE_PRIVATE)
        return prefs.getString("project_name", getString(R.string.title_project)) ?: getString(R.string.title_project)
    }

    // ---- buttons --------------------------------------------------------

    private fun button(label: Int, onClick: () -> Unit): Button =
        Button(this).apply {
            setText(label)
            setOnClickListener { onClick() }
        }

    private fun dp(v: Int): Int = (v * resources.displayMetrics.density).toInt()

    // ---- engine ---------------------------------------------------------

    private fun startEngine() {
        daemon.start { ok, why, port, token ->
            runOnUiThread {
                if (!ok) {
                    status?.text = getString(R.string.editor_engine_failed, why)
                    return@runOnUiThread
                }
                status?.text = getString(R.string.editor_listening, port)
                client.setToken(token)
                client.connect("[IP_ADDRESS]", port) { connected, err ->
                    runOnUiThread {
                        if (!connected) {
                            status?.text = getString(R.string.editor_engine_failed, err)
                            return@runOnUiThread
                        }
                        status?.text = getString(R.string.editor_ready)
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
                    status?.text = getString(R.string.editor_call_failed, text)
                    return@runOnUiThread
                }
                val clips = try {
                    org.json.JSONArray(text)
                } catch (e: Exception) {
                    org.json.JSONArray()
                }
                for (i in 0 until clips.length()) {
                    val clip = clips.optJSONObject(i) ?: continue
                    val labelStr = (clip.optString("name").ifEmpty { clip.optString("path", "?") }) + " (" + (clip.optLong("duration_ms", 5000) / 1000) + "s)";
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

        // Folder picker result: user picked a directory (Downloads, Movies, etc.)
        if (requestCode == REQUEST_PICK_FOLDER) {
            if (resultCode == RESULT_OK) {
                data?.data?.let { uri ->
                    // Hold on to this URI across restarts
                    contentResolver.takePersistableUriPermission(
                        uri,
                        Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
                    )
                    pickedFolderUri = uri
                    // Show a friendly name: last path segment, or the URI itself
                    val display = uri.lastPathSegment ?: uri.toString()
                    newLocation?.setText(display)
                    status?.text = getString(R.string.editor_folder_picked, display)
                }
            }
            return
        }

        if (requestCode != REQUEST_PICK || resultCode != RESULT_OK) return
        val uri: Uri = data?.data ?: return

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
        val kind = kindFor(uri) ?: sniffKind(target)
        kind?.let { args.put("kind", it) }
        client.call("add_media", args) { ok, text ->
            runOnUiThread {
                if (!ok) {
                    Toast.makeText(this, text, Toast.LENGTH_LONG).show()
                    return@runOnUiThread
                }
                val mediaId = try { org.json.JSONObject(text).optString("media_id") } catch (e: Exception) { "" }
                if (mediaId.isNotEmpty()) {
                    val clipArgs = org.json.JSONObject()
                        .put("media_id", mediaId)
                        .put("track_index", 0)
                        .put("start_ms", 0)
                    client.call("add_clip_to_timeline", clipArgs) { ok2, text2 ->
                        runOnUiThread {
                            if (!ok2) {
                                Toast.makeText(this, text2, Toast.LENGTH_LONG).show()
                            }
                            Toast.makeText(this, R.string.editor_imported, Toast.LENGTH_SHORT).show()
                            refresh()
                        }
                    }
                } else {
                    Toast.makeText(this, R.string.editor_imported, Toast.LENGTH_SHORT).show()
                    refresh()
                }
            }
        }
    }

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
        val stem: String
        val ext: String
        if (clean != null && dot > 0) {
            stem = clean.take(dot)
            ext = clean.substring(dot)
        } else {
            stem = clean ?: "import-${System.currentTimeMillis()}"
            ext = extensionFor(uri)
        }
        return "${System.currentTimeMillis()}-$stem$ext"
    }

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
            startsAt(4, 0x66, 0x74, 0x79, 0x70) -> "video"
            starts(0x1a, 0x45, 0xdf, 0xa3) -> "video"
            starts(0x52, 0x49, 0x46, 0x46) -> "video"
            starts(0x4f, 0x67, 0x67, 0x53) -> "audio"
            starts(0x49, 0x44, 0x33) -> "audio"
            starts(0xff, 0xfb) || starts(0xff, 0xf3) -> "audio"
            starts(0x66, 0x4c, 0x61, 0x43) -> "audio"
            starts(0x89, 0x50, 0x4e, 0x47) -> "image"
            starts(0xff, 0xd8) -> "image"
            starts(0x47, 0x49, 0x46, 0x38) -> "image"
            starts(0x42, 0x4d) -> "image"
            else -> null
        }
    }

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
        status?.text = getString(R.string.editor_rendering)
        client.call("preview_frame", JSONObject().put("width", 540)) { ok, text ->
            runOnUiThread {
                if (!ok || text.isNullOrEmpty()) {
                    status?.text = getString(R.string.editor_call_failed, text)
                    return@runOnUiThread
                }
                try {
                    val bytes = Base64.decode(text, Base64.DEFAULT)
                    val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
                    if (bitmap == null) {
                        status?.text = getString(R.string.editor_no_frame)
                        return@runOnUiThread
                    }
                    preview.setImageBitmap(bitmap)
                    status?.text = getString(R.string.editor_ready)
                } catch (e: Exception) {
                    status?.text = getString(R.string.editor_call_failed, e.message ?: "")
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
            Log.e(TAG, "could not share ${file.absolutePath}", e)
            Toast.makeText(this, file.absolutePath, Toast.LENGTH_LONG).show()
        }
    }

    private fun putInTheGallery(file: File): android.net.Uri? {
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
