package domica.caprust.remote

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import org.json.JSONArray
import org.json.JSONObject
import java.io.BufferedReader
import java.io.BufferedWriter
import java.io.InputStreamReader
import java.io.OutputStreamWriter
import java.net.InetSocketAddress
import java.net.Socket
import java.util.concurrent.Executors

/**
 * Talks JSON-RPC to a CapRust instance running on another machine.
 *
 * The whole conversation is newline-delimited JSON on one socket, the
 * same shape the desktop MCP server uses over stdio. That is why this
 * app needs no dependency: org.json ships with Android.
 *
 * Request/response runs on one serial thread, which is what keeps
 * replies matched to their requests.
 *
 * Connection setup deliberately does NOT run there. A screen-off
 * leaves a read blocked waiting for a reply that will never arrive, and
 * on a shared serial executor that stalls every queued Connect behind
 * it -- the button appears to do nothing. Connect gets its own thread
 * so it can close the socket and unblock the stuck read.
 *
 * `generation` fences the two. A reply from a socket that has since
 * been replaced is dropped rather than delivered to whichever call
 * happens to be waiting now.
 */
class Client(private val ctx: Context) {

    private val io = Executors.newSingleThreadExecutor()
    private val net = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())

    private var socket: Socket? = null
    private var writer: BufferedWriter? = null
    private var reader: BufferedReader? = null
    private var nextId = 1

    @Volatile
    private var generation = 0

    /** Token from a completed pairing. Persisted so a reconnect does not
     *  need the six-digit code again. */
    var token: String? = null
        private set

    /** Lines shown in the in-app log. Also the only diagnostic surface:
     *  the app has no adb access on a normal install, so if something
     *  goes wrong this is what the user can read back. */
    val log = ArrayList<String>()

    /** Called with every raw response line, on the main thread. */
    var onEvent: ((String) -> Unit)? = null

    private fun post(f: () -> Unit) = main.post(f)

    private fun note(line: String) {
        synchronized(log) {
            log.add(line)
            // A runaway server should not grow this without bound.
            while (log.size > 500) log.removeAt(0)
        }
        post { onEvent?.invoke(line) }
    }

    /** Open the socket. Returns once the TCP connection is up; the
     *  handshake is a separate [call] so a wrong code reports itself
     *  rather than looking like a network failure. */
    fun connect(host: String, port: Int, then: (Boolean, String) -> Unit) = net.execute {
        // Bump first: any reply still in flight on the old socket is
        // now stale and will be discarded when it arrives.
        val gen = ++generation
        try {
            closeQuietly()
            val s = Socket()
            // Short connect timeout: a wrong IP should say so in a few
            // seconds rather than hanging the button forever.
            s.connect(InetSocketAddress(host, port), 4000)
            s.soTimeout = 8000
            if (gen != generation) {
                // A newer connect won the race while we were dialling.
                s.close()
                return@execute
            }
            socket = s
            writer = BufferedWriter(OutputStreamWriter(s.getOutputStream(), Charsets.UTF_8))
            reader = BufferedReader(InputStreamReader(s.getInputStream(), Charsets.UTF_8))
            post { then(true, "") }
        } catch (e: Exception) {
            closeQuietly()
            val why = e.message ?: e.javaClass.simpleName
            post { then(false, why) }
        }
    }

    /**
     * Trade a pairing code for the long-lived token.
     *
     * `auth` is one of the few methods reachable without a token, so
     * this has to work before one exists.
     */
    fun pair(code: String, then: (Boolean, String) -> Unit) {
        val params = JSONObject().put("code", code.trim())
        raw("auth", params) { ok, resp, err ->
            if (!ok) {
                post { then(false, err) }
                return@raw
            }
            val t = resp.optString("token")
            if (t.isNullOrEmpty()) {
                post { then(false, "no token in response") }
                return@raw
            }
            token = t
            ctx.getSharedPreferences("caprust", Context.MODE_PRIVATE)
                .edit().putString("token", t).apply()
            post { then(true, t) }
        }
    }

    /**
     * Call a tool.
     *
     * The token rides inside params, not a header: the server speaks
     * newline-delimited JSON with no request headers, and a client that
     * invented one would not work against the stdio transport.
     */
    fun call(name: String, args: JSONObject = JSONObject(), then: (Boolean, String) -> Unit) {
        val params = JSONObject().put("name", name).put("arguments", args)
        token?.let { params.put("token", it) }
        raw("tools/call", params) { ok, resp, err ->
            if (!ok) {
                post { then(false, err) }
                return@raw
            }
            if (resp.optBoolean("isError", false)) {
                val msg = resp.optJSONArray("content")?.optJSONObject(0)
                    ?.optString("text").orEmpty()
                post { then(false, msg.ifEmpty { err }) }
                return@raw
            }
            val text = resp.optJSONArray("content")?.optJSONObject(0)
                ?.optString("text").orEmpty()
            post { then(true, text) }
        }
    }

    /** Low-level send/receive. Kept separate so [pair] can reach a
     *  method that is not a tool call. */
    private fun raw(method: String, params: JSONObject, done: (Boolean, JSONObject, String) -> Unit) =
        io.execute {
            // Captured before the send: if the socket is replaced while
            // this is waiting, the reply belongs to a connection nobody
            // is using any more.
            val gen = generation
            try {
                val w = writer ?: throw IllegalStateException("not connected")
                val r = reader ?: throw IllegalStateException("not connected")
                val req = JSONObject()
                    .put("jsonrpc", "2.0")
                    .put("id", nextId++)
                    .put("method", method)
                    .put("params", params)
                w.write(req.toString())
                w.newLine()
                w.flush()
                note("> $req")
                val line = r.readLine() ?: throw IllegalStateException("connection closed")
                note("< $line")
                if (gen != generation) {
                    note("~ reply dropped: connection was replaced")
                    return@execute
                }
                val v = JSONObject(line)
                if (v.has("error")) {
                    val e = v.getJSONObject("error")
                    done(false, v, "(${e.optInt("code")}) ${e.optString("message")}")
                } else {
                    done(true, v.optJSONObject("result") ?: JSONObject(), "")
                }
            } catch (e: Exception) {
                val why = e.message ?: e.javaClass.simpleName
                note("! $why")
                if (gen != generation) return@execute
                done(false, JSONObject(), why)
            }
        }

    fun disconnect() = net.execute {
        generation++
        closeQuietly()
        post { onEvent?.invoke("-- disconnected") }
    }

    private fun closeQuietly() {
        try { socket?.close() } catch (_: Exception) {}
        socket = null
        reader = null
        writer = null
    }

    /** Reload a persisted token so a reconnect is automatic. */
    fun loadToken() {
        token = ctx.getSharedPreferences("caprust", Context.MODE_PRIVATE)
            .getString("token", null)
    }
}

/** One activity, one screen, built in code.
 *
 *  XML layouts would need a resource compiler and a view-binding
 *  setup for a UI this small, and a linear stack of views is easier to
 *  follow here than a nested layout file would be. */
class MainActivity : android.app.Activity() {

    private lateinit var client: Client
    private lateinit var status: TextView
    private lateinit var clipList: LinearLayout
    private lateinit var logView: TextView
    private lateinit var hostBox: EditText
    private lateinit var portBox: EditText
    private lateinit var codeBox: EditText

    /** Milliseconds used by Split and Move. Prefilled with the selected
     *  clip's midpoint so the common case is still one tap. */
    private lateinit var timeBox: EditText

    /** Detail panel for the selected clip, landscape only. */
    private var selectedView: TextView? = null

    /** Drawn timeline map. */
    private lateinit var timeline: TimelineView

    /**
     * Whether the raw traffic log is shown.
     *
     * A setting, not a preference: on a small phone the log eats a
     * quarter of the screen for something only useful when something is
     * wrong. Defaults on, because the log is the only diagnostic
     * without adb.
     */
    private var logOn: Boolean
        get() = getSharedPreferences("caprust", MODE_PRIVATE)
            .getBoolean("logOn", true)
        set(v) = getSharedPreferences("caprust", MODE_PRIVATE)
            .edit().putBoolean("logOn", v).apply()

    /** Cached clip fields for the selected row, so the sheet and the
     *  detail panel do not have to re-query. */
    private val clipInfo = HashMap<String, JSONObject>()

    /** Preset ids per kind ("effect", "filter", "transition"), fetched
     *  once from the server. See [showPresetPicker]. */
    private val assetsCache = HashMap<String, List<String>>()

    /** Label of the log toggle button; its text reflects the action. */
    private lateinit var logToggleLabel: Button

    /** Theme button, kept so its label can follow the current choice
     *  without rebuilding the whole view. */
    private lateinit var themeButton: Button

    /** Project duration from the last summary, so the timeline can
     *  scale to fit. Cached because the clip list arrives separately. */
    private var lastDurationMs: Long = 1

    /** Clip id -> display name, rebuilt on every refresh. */
    private val clipNames = LinkedHashMap<String, String>()
    private var selectedClip: String? = null
    private var connected = false

    /**
     * Whether the user wants the link up.
     *
     * Persisted, not just a field: Android may destroy and recreate
     * the activity while the screen is off, which leaves a fresh
     * instance with no socket and an empty clip list. Reading this back
     * on resume is what reconnects instead of stranding the user on a
     * blank screen.
     */
    private var wanted: Boolean
        get() = getSharedPreferences("caprust", MODE_PRIVATE).getBoolean("wanted", false)
        set(v) = getSharedPreferences("caprust", MODE_PRIVATE).edit().putBoolean("wanted", v).apply()

    override fun onCreate(savedInstanceState: android.os.Bundle?) {
        super.onCreate(savedInstanceState)
        applyTheme(themeChoice(), apply = false)
        client = Client(this).also { it.loadToken() }
        client.onEvent = { renderLog() }
        setContentView(build())
        refresh()
    }

    // ---- theme ----

    /**
     * "system", "light" or "dark".
     *
     * Persisted because a theme the user picked once should not revert
     * on the next launch.
     */
    private fun themeChoice(): String =
        getSharedPreferences("caprust", MODE_PRIVATE).getString("theme", "system") ?: "system"

    /**
     * Apply a theme by resource id and recreate.
     *
     * setTheme + recreate rather than AppCompatDelegate, which would
     * pull in the support library this app deliberately does not have.
     */
    private fun applyTheme(choice: String, apply: Boolean = true) {
        val id = when (choice) {
            "light" -> R.style.Theme_CapRustRemote_Light
            "dark" -> R.style.Theme_CapRustRemote_Dark
            else -> when (resources.configuration.uiMode and
                android.content.res.Configuration.UI_MODE_NIGHT_MASK) {
                android.content.res.Configuration.UI_MODE_NIGHT_YES ->
                    R.style.Theme_CapRustRemote_Dark
                else -> R.style.Theme_CapRustRemote_Light
            }
        }
        if (theme == id) return
        setTheme(id)
        if (apply) recreate()
    }

    /** Cycle light -> dark -> system. One button instead of a settings
     *  dialog: three options do not need a list. */
    private fun cycleTheme() {
        val next = when (themeChoice()) {
            "light" -> "dark"
            "dark" -> "system"
            else -> "light"
        }
        getSharedPreferences("caprust", MODE_PRIVATE).edit().putString("theme", next).apply()
        applyTheme(next)
    }

    private fun themeLabel(): String = when (themeChoice()) {
        "light" -> getString(R.string.theme_light)
        "dark" -> getString(R.string.theme_dark)
        else -> getString(R.string.theme_system)
    }

    /**
     * Re-establish the socket when coming back to the foreground.
     *
     * Screen off tears the connection down. It does it silently: no call
     * happens while the app is asleep, so nothing notices and
     * `connected` is still true on resume. Guarding on that flag meant
     * the app came back looking alive and did nothing, which is why
     * Connect still had to be pressed. Checking `wanted` alone is the
     * only correct test -- the only way to know a socket survived
     * sleep is to reopen it, and a wasted reconnect to a healthy
     * server costs milliseconds.
     */
    override fun onResume() {
        super.onResume()
        if (!wanted) return
        val host = sharedPrefsHost()
        if (host.isEmpty()) return
        status.text = "Reconnecting to $host ..."
        client.connect(host, sharedPrefsPort().toIntOrNull() ?: 4711) { ok, err ->
            connected = ok
            status.text = if (ok) "Connected to $host" else "Reconnect failed: $err"
            if (ok) refresh()
        }
    }

    private fun dp(v: Int) = (v * resources.displayMetrics.density).toInt()

    private fun build(): View {
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(12), dp(12), dp(12), dp(12))
        }

        root.addView(TextView(this).apply {
            text = "CapRust Remote"
            textSize = 20f
            setTypeface(typeface, android.graphics.Typeface.BOLD)
        })

        status = TextView(this).apply { textSize = 13f }
        root.addView(status)

        // ---- connection row ----
        val conn = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        hostBox = EditText(this).apply {
            hint = getString(R.string.lbl_host)
            // On one line: breaking after `=` makes Kotlin read the
            // first operand as a statement and then trip over `or`.
            inputType = android.text.InputType.TYPE_CLASS_TEXT or
                android.text.InputType.TYPE_TEXT_VARIATION_URI
            setText(sharedPrefsHost())
        }
        portBox = EditText(this).apply {
            hint = getString(R.string.lbl_port)
            inputType = android.text.InputType.TYPE_CLASS_NUMBER
            setText(sharedPrefsPort())
        }
        conn.addView(hostBox, LinearLayout.LayoutParams(0, dp(48), 3f))
        conn.addView(portBox, LinearLayout.LayoutParams(0, dp(48), 1f))
        conn.addView(btn(getString(R.string.btn_connect)) { doConnect() },
            LinearLayout.LayoutParams(dp(110), dp(48)))
        root.addView(conn)

        // ---- pairing row ----
        val pairRow = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        codeBox = EditText(this).apply {
            hint = getString(R.string.lbl_pair_code)
            inputType = android.text.InputType.TYPE_CLASS_NUMBER
        }
        pairRow.addView(codeBox, LinearLayout.LayoutParams(0, dp(48), 2f))
        pairRow.addView(btn(getString(R.string.btn_pair)) { doPair() },
            LinearLayout.LayoutParams(0, dp(48), 1f))
        root.addView(pairRow)

        // ---- actions ----
        val actions = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        actions.addView(btn(getString(R.string.btn_refresh)) { refresh() })
        actions.addView(btn(getString(R.string.btn_undo)) { client.call("undo") { ok, _ -> if (ok) refresh() } })
        actions.addView(btn(getString(R.string.btn_redo)) { client.call("redo") { ok, _ -> if (ok) refresh() } })
        actions.addView(btn(getString(R.string.btn_save)) { client.call("save_project") { ok, _ -> if (ok) refresh() } })
        root.addView(actions)

        val actions2 = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        actions2.addView(btn(getString(R.string.btn_disconnect)) { doDisconnect() })
        // Label reflects the current choice, so it doubles as the
        // indicator. Three options do not need a dialog.
        themeButton = btn(themeLabel()) { cycleTheme() }
        actions2.addView(themeButton)
        root.addView(actions2)

        // ---- time input ----
        // Split used to happen at the midpoint, which is safe but not
        // what anyone means. One field serves both Split and Move, and
        // selecting a clip prefills it with that clip's midpoint so the
        // common case is still one tap.
        val timeRow = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        timeRow.addView(TextView(this).apply {
            text = getString(R.string.lbl_time_ms)
            textSize = 12f
        })
        timeBox = EditText(this).apply {
            hint = "ms"
            inputType = android.text.InputType.TYPE_CLASS_NUMBER
        }
        timeRow.addView(timeBox, LinearLayout.LayoutParams(0, dp(44), 1f))
        root.addView(timeRow)

        root.addView(TextView(this).apply {
            text = getString(R.string.title_timeline)
            setTypeface(typeface, android.graphics.Typeface.BOLD)
            setPadding(0, dp(8), 0, 0)
        })

        timeline = TimelineView(this).apply {
            onPick = { id ->
                selectedClip = id
                timeline.setSelected(id)
                selectedView?.text = clipInfo[id]?.let { describe(id, it) } ?: ""
                showClipSheet()
            }
        }
        root.addView(timeline, LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, dp(150)))

        // ---- presets ----
        // Three lists, no preview. A preview needs frames shipped to
        // the phone; the ids are enough to make the edit and the desktop
        // still shows the result on its own preview.
        val presets = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        presets.addView(btn(getString(R.string.btn_effects)) { showPresetPicker("effect") })
        presets.addView(btn(getString(R.string.btn_filters)) { showPresetPicker("filter") })
        presets.addView(btn(getString(R.string.btn_transitions)) { showPresetPicker("transition") })
        root.addView(presets)

        root.addView(TextView(this).apply {
            text = getString(R.string.title_clips)
            setTypeface(typeface, android.graphics.Typeface.BOLD)
            setPadding(0, dp(8), 0, 0)
        })

        val clipScroll = ScrollView(this)
        clipList = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
        }
        clipScroll.addView(clipList, ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        root.addView(clipScroll, LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, 0, 3f))

        // ---- log ----
        // Built here, attached in one place further down: a View has a
        // single parent, so attaching it to both columns would throw.
        val logScroll = ScrollView(this)
        logView = TextView(this).apply {
            textSize = 10f
            setTypeface(android.graphics.Typeface.MONOSPACE)
            setTextIsSelectable(true)
        }
        logScroll.addView(logView)
        logView.visibility = if (logOn) View.VISIBLE else View.GONE

        if (resources.configuration.orientation ==
            android.content.res.Configuration.ORIENTATION_PORTRAIT) {
            root.addView(logToggle())
            root.addView(logScroll, LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                if (logOn) dp(140) else dp(1)))
            return root
        }

        // Landscape on a tablet was throwing away half the screen on a
        // single column. Two columns: list on the left, selected clip
        // and the log on the right.
        //
        // The log goes in exactly one place. A View has one parent, so
        // adding it to both columns throws at inflate time.
        val wide = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        wide.addView(root, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 1f))
        val side = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(8), 0, 0, 0)
        }
        side.addView(
            selectedInfo(),
            LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            )
        )
        side.addView(logToggle())
        side.addView(
            logScroll,
            LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f
            )
        )
        wide.addView(side, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 1f))
        return wide
    }

    private fun logToggle() = btn("", {}) .also { b ->
        logToggleLabel = b
        b.text = if (logOn) getString(R.string.btn_hide_log) else getString(R.string.btn_show_log)
        b.setOnClickListener { toggleLog() }
    }

    private fun toggleLog() {
        logOn = !logOn
        logView.visibility = if (logOn) View.VISIBLE else View.GONE
        // Keep capturing either way: a hidden log is still the
        // diagnostic when the user goes looking for it.
        logToggleLabel.text = if (logOn) "Hide log" else "Show log"
    }

    /** Detail panel for the selected clip. Only shown in landscape;
     *  portrait has no room and the bottom sheet carries this. */
    private fun selectedInfo(): TextView {
        selectedView = TextView(this).apply {
            textSize = 12f
            setTypeface(android.graphics.Typeface.MONOSPACE)
            setPadding(0, dp(4), 0, dp(4))
        }
        return selectedView!!
    }

    private fun btn(label: String, onClick: () -> Unit) = Button(this).apply {
        text = label
        textSize = 12f
        setOnClickListener { onClick() }
        // Buttons default to all-caps; these are short labels and the
        // mixed case reads better next to the monospace log.
        isAllCaps = false
        gravity = Gravity.CENTER
    }

    private fun sharedPrefsHost(): String =
        getSharedPreferences("caprust", MODE_PRIVATE).getString("host", "") ?: ""

    private fun sharedPrefsPort(): String =
        getSharedPreferences("caprust", MODE_PRIVATE).getString("port", "4711") ?: "4711"

    private fun doConnect() {
        val host = hostBox.text.toString().trim()
        val port = portBox.text.toString().trim().toIntOrNull() ?: 4711
        if (host.isEmpty()) {
            status.text = "Enter the computer's IP address"
            return
        }
        status.text = "Connecting to $host:$port ..."
        client.connect(host, port) { ok, err ->
            connected = ok
            // Remembered separately from `connected`: this is the
            // user's intent, and a failed attempt should still be
            // retried on the next resume.
            wanted = true
            if (!ok) {
                status.text = "Failed: $err"
            } else {
                getSharedPreferences("caprust", MODE_PRIVATE).edit()
                    .putString("host", host).putString("port", "$port").apply()
                status.text = "Connected to $host:$port"
                refresh()
            }
        }
    }

    private fun doPair() {
        val code = codeBox.text.toString().trim()
        if (code.isEmpty()) {
            status.text = "Enter the code shown in CapRust on the computer"
            return
        }
        status.text = "Pairing ..."
        client.pair(code) { ok, msg ->
            status.text = if (ok) "Paired" else "Pairing failed: $msg"
            if (ok) {
                codeBox.text.clear()
                refresh()
            }
        }
    }

    private fun doDisconnect() {
        client.disconnect()
        connected = false
        // Explicit disconnect, so stop reconnecting behind their back.
        wanted = false
        status.text = "Disconnected"
        clipList.removeAllViews()
    }

    /** Pull the summary and the clip list. Both are read-only, so a
     *  token-less client fails here rather than looking like a crash. */
    private fun refresh() {
        if (!connected) {
            // Without this the screen comes back from sleep completely
            // blank, with nothing to say why.
            // childCount, not hasViews(): the latter is an androidx
            // KTX extension and this app has no androidx dependency.
            if (clipList.childCount == 0) {
                clipList.removeAllViews()
                val note = TextView(this)
                note.text = if (wanted) "Reconnecting ..." else "Tap Connect to reach CapRust"
                clipList.addView(note)
            }
            return
        }
        client.call("get_project_summary") { ok, text ->
            if (!ok) {
                runOnUiThread { onCallFailed(text) }
                return@call
            }
            lastDurationMs = runCatching {
                JSONObject(text).optLong("total_duration_ms", 1L)
            }.getOrDefault(1L)
            runOnUiThread { status.text = summarise(text) }
        }
        client.call("list_clips") { ok, text ->
            if (!ok) {
                runOnUiThread { status.text = text }
                return@call
            }
            runOnUiThread { renderClips(text) }
        }
    }

    /**
 * A call failed.
 *
 * A dead socket is the common case -- screen off, Wi-Fi roam, sleep --
 * and it reads as a mysterious tool error otherwise. Mark the link
 * down so the next resume reconnects, and say which it was.
 */
    private fun onCallFailed(message: String) {
        val linkDown = message.contains("not connected", true) ||
            message.contains("connection", true) ||
            message.contains("closed", true)
        if (linkDown) {
            connected = false
            status.text = "Connection lost. It will reconnect when you return."
        } else {
            status.text = message
        }
    }

    /** The summary arrives as a pretty-printed JSON string inside the
     *  tool result. Parse it rather than regexing: the field set can
     *  grow and the parse is already there. */
    private fun summarise(text: String): String = try {
        val j = JSONObject(text)
        buildString {
            append(j.optString("name", "?"))
            append("  ")
            append(j.optInt("clip_count"))
            append(" clips, ")
            append(j.optInt("media_count"))
            append(" media, ")
            append(j.optLong("total_duration_ms") / 1000)
            append("s")
        }
    } catch (_: Exception) {
        text.take(120)
    }

    private fun renderClips(text: String) {
        clipList.removeAllViews()
        clipNames.clear()
        clipInfo.clear()
        val arr = try {
            org.json.JSONArray(text)
        } catch (_: Exception) {
            // The first version built a TextView, dropped it, and then
            // added `this` -- the Activity. Bind it to a name.
            val fallback = TextView(this)
            fallback.text = text.take(200)
            clipList.addView(fallback)
            timeline.setClips(org.json.JSONArray(), 1)
            return
        }
        // Feed the drawn map from the same array the list uses, so the
        // two can never disagree about what is on the timeline.
        timeline.setClips(arr, lastDurationMs)
        timeline.setSelected(selectedClip)
        if (arr.length() == 0) {
            val empty = TextView(this)
            empty.setText(getString(R.string.no_clips))
            clipList.addView(empty)
            return
        }
        for (i in 0 until arr.length()) {
            val c = arr.getJSONObject(i)
            val id = c.optString("id")
            // list_clips reports `kind`, not `type`, and carries no
            // display name. Both were wrong in the first version of
            // this screen, which showed an empty row per clip.
            val kind = c.optString("kind", "clip")
            val startSec = c.optLong("start_time_ms") / 1000
            val label = "$kind  @${startSec}s"
            clipNames[id] = label
            clipInfo[id] = c
            val row = btn(label) {
                selectedClip = id
                dimOthers(id)
                // Prefill the time field with this clip's midpoint, so
                // Split is still one tap for the common case while
                // staying adjustable.
                if (::timeBox.isInitialized) {
                    val mid = (c.optLong("start_time_ms") + c.optLong("duration_ms") / 2)
                    timeBox.setText(mid.toString())
                }
                selectedView?.text = describe(id, c)
                showClipSheet()
            }
            row.tag = id
            clipList.addView(row)
        }
    }

    /**
 * Bottom sheet of actions for the selected clip.
 *
 * Every button here maps to a tool the server already has; none of it
 * is new backend. Grouped in one place rather than as a row of
 * buttons because there are more actions than fit across a phone.
 */
    private fun showClipSheet() {
        val id = selectedClip
        if (id == null) {
            status.text = getString(R.string.msg_tap_clip)
            return
        }
        val info = clipInfo[id] ?: JSONObject()
        val start = info.optLong("start_time_ms")
        val dur = info.optLong("duration_ms")
        val kind = info.optString("kind", "clip")

        android.app.Dialog(this).apply {
            setTitle(getString(R.string.sheet_title))
            val col = LinearLayout(this@MainActivity).apply {
                orientation = LinearLayout.VERTICAL
                setPadding(dp(16), dp(8), dp(16), dp(16))
            }
            fun row(label: String, body: () -> Unit) = col.addView(btn(label) {
                body()
                dismiss()
            })

            row(getString(R.string.act_split)) { splitAt(readTimeOr(start + dur / 2)) }
            row(getString(R.string.act_move)) { moveTo(readTimeOr(start)) }
            row(getString(R.string.act_vol_up)) {
                setVolume(info.optDouble("volume_db", 0.0) + 6.0)
            }
            row(getString(R.string.act_vol_down)) {
                setVolume(info.optDouble("volume_db", 0.0) - 6.0)
            }
            row(getString(R.string.act_speed_up)) { setSpeed(info.optDouble("speed", 1.0) * 2.0) }
            row(getString(R.string.act_speed_half)) { setSpeed(info.optDouble("speed", 1.0) / 2.0) }
            row(getString(R.string.act_fade)) { setFade() }
            if (kind.contains("Audio", true) || kind.contains("Video", true)) {
                row(getString(R.string.act_media)) { client.call("list_media") { ok, t ->
                    runOnUiThread { status.text = if (ok) t.take(300) else t }
                } }
            }

            setContentView(col)
            show()
        }
    }

    /**
     * Pick a transition, effect or filter for the selected clip.
     *
     * Ids come from `list_assets` rather than being hardcoded here. The
     * desktop owns the preset table; a copy on the phone would list
     * effects that no longer exist and silently omit ones that do.
     *
     * Cached after the first fetch -- these tables do not change while
     * the app is open, and re-querying on every button press would put
     * a round trip in front of every tap.
     */
    private fun showPresetPicker(kind: String) {
        val id = selectedClip
        if (id == null) {
            status.text = getString(R.string.msg_tap_clip)
            return
        }
        assetsCache[kind]?.let { return showPresetList(kind, id, it) }
        client.call("list_assets") { ok, text ->
            runOnUiThread {
                if (!ok) {
                    status.text = text
                    return@runOnUiThread
                }
                val arr = try {
                    JSONObject(text).optJSONArray(kind) ?: JSONArray()
                } catch (_: Exception) {
                    JSONArray()
                }
                if (arr.length() == 0) {
                    status.text = getString(R.string.no_presets)
                    return@runOnUiThread
                }
                val ids = ArrayList<String>(arr.length())
                for (i in 0 until arr.length()) ids.add(arr.getString(i))
                assetsCache[kind] = ids
                showPresetList(kind, id, ids)
            }
        }
    }

    private fun showPresetList(kind: String, clipId: String, ids: List<String>) {
        android.app.AlertDialog.Builder(this)
            .setTitle(getString(
                when (kind) {
                    "effect" -> R.string.btn_effects
                    "filter" -> R.string.btn_filters
                    else -> R.string.btn_transitions
                }
            ))
            .setItems(ids.toTypedArray()) { _, which ->
                applyPreset(clipId, kind, ids[which])
            }
            .setNegativeButton(android.R.string.cancel, null)
            .show()
    }

    private fun applyPreset(clipId: String, kind: String, presetId: String) {
        client.call(
            "apply_preset",
            JSONObject()
                .put("clip_id", clipId)
                .put("kind", kind)
                .put("preset_id", presetId)
        ) { ok, text ->
            runOnUiThread {
                if (!ok) {
                    status.text = text
                } else {
                    status.text = getString(R.string.msg_applied, presetId)
                    refresh()
                }
            }
        }
    }

    /** The typed time, or a fallback when the field is empty or not a
     *  number. Never silently uses the fallback for a bad value: a
     *  split at the wrong place is worse than one at the midpoint. */
    private fun readTimeOr(fallback: Long): Long =
        if (::timeBox.isInitialized) timeBox.text.toString().trim().toLongOrNull() ?: fallback
        else fallback

    /** Split a clip at an explicit position.
     *
     *  The argument is `at_ms`, matching SplitClipCommand. `time_ms`
     *  was silently ignored as a missing parameter in the first
     *  version.
     */
    private fun splitAt(at: Long) {
        val id = selectedClip ?: return
        client.call("split_clip", JSONObject().put("clip_id", id).put("at_ms", at)) { ok, msg ->
            runOnUiThread {
                status.text = if (ok) "Split at ${at}ms" else "Split failed: $msg"
                if (ok) refresh()
            }
        }
    }

    /** Move a clip to a start position.
     *
     *  The arguments are `to_ms` and the optional `to_track`, checked
     *  against the server rather than guessed. `to_track` is left out
     *  on purpose: omitting it keeps the clip where it is, which is
     *  what Move means here.
     */
    private fun moveTo(at: Long) {
        val id = selectedClip ?: return
        client.call("move_clip", JSONObject().put("clip_id", id).put("to_ms", at)) { ok, msg ->
            runOnUiThread {
                status.text = if (ok) "Moved to ${at}ms" else "Move failed: $msg"
                if (ok) refresh()
            }
        }
    }

    private fun setVolume(db: Double) {
        val id = selectedClip ?: return
        client.call("set_clip_volume", JSONObject().put("clip_id", id).put("db", db)) { ok, msg ->
            runOnUiThread {
                status.text = if (ok) "Volume $db dB" else "Volume failed: $msg"
                if (ok) refresh()
            }
        }
    }

    private fun setSpeed(speed: Double) {
        val id = selectedClip ?: return
        // The slider is 0.25..4; halving a 0.25 would ask for 0.125,
        // which the panel cannot represent.
        val clamped = speed.coerceIn(0.25, 4.0)
        client.call("set_clip_speed", JSONObject().put("clip_id", id).put("speed", clamped)) { ok, msg ->
            runOnUiThread {
                status.text = if (ok) "Speed ${clamped}x" else "Speed failed: $msg"
                if (ok) refresh()
            }
        }
    }

    private fun setFade() {
        val id = selectedClip ?: return
        val args = JSONObject().put("clip_id", id).put("fade_in_ms", 1000).put("fade_out_ms", 1000)
        client.call("set_clip_fade", args) { ok, msg ->
            runOnUiThread {
                status.text = if (ok) "Fade 1s in and out" else "Fade failed: $msg"
                if (ok) refresh()
            }
        }
    }

    /** One-clip summary for the landscape detail panel. */
    private fun describe(id: String, c: JSONObject): String = buildString {
        appendLine("id      ${id.take(8)}")
        appendLine("kind    ${c.optString("kind")}")
        appendLine("track   ${c.optInt("track_index")}")
        appendLine("start   ${c.optLong("start_time_ms")} ms")
        appendLine("length  ${c.optLong("duration_ms")} ms")
        appendLine("speed   ${c.optDouble("speed", 1.0)}x")
        append("volume  ${c.optDouble("volume_db", 0.0)} dB")
    }

    /** Dim every clip row except the selected one.
     *
     *  A plain index loop, not `childrenSequence()`: that is an
     *  androidx.core KTX extension and this app deliberately has no
     *  androidx dependency.
     */
    private fun dimOthers(selected: String) {
        for (i in 0 until clipList.childCount) {
            val child = clipList.getChildAt(i) as? Button ?: continue
            child.setAlpha(if (child.tag == selected) 1f else 0.6f)
        }
    }

    private fun renderLog() {
        val text = synchronized(client.log) { client.log.joinToString("\n") }
        logView.text = text
        logView.post { (logView.parent as? ScrollView)?.fullScroll(View.FOCUS_DOWN) }
    }
}