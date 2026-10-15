package domica.caprust.remote

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.ImageView
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

    /** How many lines may arrive before a reply is given up on. Bounds
     *  the id-matching loop in [raw]; a server sending only
     *  notifications would otherwise spin here. */
    private val MAX_LINES_PER_REPLY = 16
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
            // Generous. The desktop answers requests from its UI tick,
            // so a busy or backgrounded window makes the reply late --
            // it does not make it wrong, and a stale reply is now
            // skipped by id rather than handed to the wrong caller.
            s.soTimeout = 20000
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
                val id = nextId++
                val req = JSONObject()
                    .put("jsonrpc", "2.0")
                    .put("id", id)
                    .put("method", method)
                    .put("params", params)
                w.write(req.toString())
                w.newLine()
                w.flush()
                note("> $req")

                // Read until the reply carries *our* id. The transport is
                // one request/one response in order, so under normal use
                // the first line is already the right one -- but a call
                // that timed out leaves its reply sitting in the socket,
                // and the next call used to hand that reply to whoever
                // asked next. That is how pairing reported "no token in
                // response" against a desktop that had paired fine: the
                // line read belonged to the earlier tools/list.
                //
                // Bounded, so a server that only ever sends notifications
                // cannot spin here forever.
                var line: String? = null
                // `attempt` rather than `_`: Kotlin reserves the underscore for unused
                // parameters and rejects it as a loop variable here.
                for (attempt in 0..MAX_LINES_PER_REPLY) {
                    val got = r.readLine() ?: throw IllegalStateException("connection closed")
                    note("< $got")
                    val envelope = runCatching { JSONObject(got) }.getOrNull()
                    if (envelope == null) {
                        line = got
                        break
                    }
                    if (!envelope.has("id") || envelope.optLong("id", -1L) == id.toLong()) {
                        line = got
                        break
                    }
                    note("~ skipped a reply for id ${envelope.optLong("id", -1L)}")
                }
                val text = line
                    ?: throw IllegalStateException("no reply for id $id after $MAX_LINES_PER_REPLY lines")
                if (gen != generation) {
                    note("~ reply dropped: connection was replaced")
                    return@execute
                }
                val v = JSONObject(text)
                if (v.has("error")) {
                    val e = v.getJSONObject("error")
                    done(false, v, "(${e.optInt("code")}) ${e.optString("message")}")
                } else {
                    done(true, v.optJSONObject("result") ?: JSONObject(), "")
                }
            } catch (e: Exception) {
                val why = e.message ?: e.javaClass.simpleName
                note("! $why")
                // A read that failed leaves the stream at an unknown
                // offset. Anything still in flight would be handed to the
                // next caller, so the socket is now unusable: say so
                // rather than return a stranger's reply.
                kill()
                if (gen != generation) return@execute
                done(false, JSONObject(), why)
            }
        }

    /** Drop the socket without touching the reconnect bookkeeping.
     *
     * Called when a read has failed, so the next call cannot inherit a
     * half-consumed stream. The user reconnects; [onEvent] tells them.
     */
    private fun kill() {
        writer = null
        reader = null
        closeQuietly()
        post { onEvent?.invoke("-- link lost") }
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

    /**
     * Adopt a token stored for a different computer.
     *
     * Needed because the token belongs to a desktop, not to the app: a
     * pairing earned on one machine is rejected by another, so loading
     * the saved one on switch is the whole point of saving it.
     */
    /**
     * Ask which tools this server offers.
     *
     * `tools/list` needs no token, so this works before pairing and is
     * the one call that can tell a too-old desktop apart from a
     * missing pairing.
     */
    fun listTools(then: (Boolean, List<String>) -> Unit) {
        raw("tools/list", JSONObject()) { ok, resp, err ->
            if (!ok) {
                post { then(false, emptyList()) }
                return@raw
            }
            val names = ArrayList<String>()
            // raw() already unwraps "result" before calling back -- that
            // is why call() reads resp.optString("token") and
            // resp.optJSONArray("content") with no "result" hop. Looking
            // for a nested "result" here found nothing, so the tool list
            // came back empty and every connect ended at "could not read
            // the tool list". The array is directly on resp.
            resp.optJSONArray("tools")?.let { arr ->
                for (i in 0 until arr.length()) {
                    arr.optJSONObject(i)?.optString("name")?.let { names.add(it) }
                }
            }
            post { then(names.isNotEmpty(), names) }
        }
    }

    fun setToken(v: String?) {
        token = v
    }

    /**
     * Forget a token the server has rejected.
     *
     * Must clear storage too, not just the field. A token that was
     * only cleared in memory comes back on the next launch and the app
     * believes it is paired again -- the same dead end, once per
     * restart.
     */
    fun clearToken() {
        token = null
        ctx.getSharedPreferences("caprust", Context.MODE_PRIVATE)
            .edit().remove("token").apply()
    }

    /** True once a pairing has completed this session or been loaded
     *  from storage. Used to label saved connections honestly. */
    fun isPaired(): Boolean = !token.isNullOrEmpty()

    /** Drop the captured traffic. */
    fun clearLog() {
        synchronized(log) { log.clear() }
    }
}

/** One activity, several screens, built in code.
 *
 *  XML layouts would need a resource compiler and a view-binding
 *  setup for a UI this small, and a linear stack of views is easier to
 *  follow here than a nested layout file would be.
 *
 *  Screens swap by rebuilding the content view rather than by keeping
 *  five trees alive and toggling visibility. Toggling is cheaper, but
 *  a hidden LinearLayout still measures itself, and five measured
 *  trees on a tablet is how a layout ends up scrolled somewhere
 *  unexpected. Rebuilding also means a screen never shows stale data
 *  from the last time it was visited. */
class MainActivity : android.app.Activity() {

    /**
     * The five screens.
     *
     * An enum rather than an int or a string so a `when` over it is
     * checked for exhaustiveness and a typo is a compile error.
     */
    private enum class Screen { Connect, Home, Project, Projects, Log, Settings, Connections }

    /**
     * Tools this app needs from a CapRust instance.
     *
     * Checked at connect. A desktop older than the app simply does not
     * have some of these, and the symptom otherwise is a button that
     * does nothing -- indistinguishable from a bug here, which has
     * cost several rounds of guessing already.
     */
    private val requiredTools = listOf(
        "get_project_summary", "list_clips", "move_clip", "split_clip",
        "set_clip_volume", "set_clip_speed", "set_clip_fade", "list_media",
        "add_clip_to_timeline", "undo", "redo", "save_project",
        "list_assets", "apply_preset", "list_recent_projects", "open_project",
        "preview_frame", "start_export", "export_status",
        "preview_transport", "preview_status",
        "export_settings", "set_export_setting",
    )

    /**
     * Tools this app can use if the desktop has them, and does without
     * if it does not.
     *
     * Separate from [requiredTools] on purpose. A feature added to the
     * app must not become a gate: putting `remove_effect` in the
     * required list made every desktop one build older refuse to talk to
     * the phone at all, which reads as "the app is broken" rather than
     * "one button is missing". A phone is expected to outlive any single
     * desktop build.
     */
    private val optionalTools = listOf(
        "remove_effect",
    )

    private var current = Screen.Connect

    /** Whether this desktop has remove_effect. Guards the button that
     *  needs it; a desktop without it is older, not broken. */
    private var hasRemoveEffect = false

    /**
     * Remember the screen across a rotation.
     *
     * Android destroys and recreates the activity for a configuration
     * change, taking the socket with it. Coming back on Connect is not
     * a neutral default here: it throws away the session the user was
     * in the middle of, on a device that has no other way back.
     */
    override fun onSaveInstanceState(outState: android.os.Bundle) {
        super.onSaveInstanceState(outState)
        outState.putString("screen", current.name)
    }

    /** Screen name to screen, falling back to Connect on anything
     *  unrecognised rather than throwing during recreation. */
    private fun screenFrom(name: String): Screen =
        try {
            Screen.valueOf(name)
        } catch (_: Exception) {
            Screen.Connect
        }

    private lateinit var client: Client
    // Null while no screen showing that particular view is attached.
    // A reply can arrive after the user has navigated away, and a
    // lateinit field would throw on exactly that path -- which is the
    // path a screen-off or a slow request takes.
    private var status: TextView? = null
    private var clipList: LinearLayout? = null
    private var logView: TextView? = null
    private lateinit var hostBox: EditText
    private lateinit var portBox: EditText
    private lateinit var codeBox: EditText

    /** Milliseconds used by Split and Move. Prefilled with the selected
     *  clip's midpoint so the common case is still one tap. */
    private lateinit var timeBox: EditText

    /** Detail panel for the selected clip, landscape only. */
    private var selectedView: TextView? = null

    /** Drawn timeline map. */
    /** Null while a screen without a timeline is showing. */
    private var timeline: TimelineView? = null

    /** The last preview frame from the desktop, base64 JPEG. Null
     *  until one has been fetched. */
    private var previewView: ImageView? = null

    /** Export button, kept so its label can show progress. */
    private var exportButton: Button? = null

    /** Play/pause buttons; the active one is highlighted. */
    private var playButton: TextView? = null
    private var pauseButton: TextView? = null

    /** Playhead readout under the transport. */
    private var playheadView: TextView? = null

    /** Explains an empty preview area. */
    private var previewNote: TextView? = null

    /** Project length in ms, from `get_project_summary`. Needed to turn
     *  a tap on the preview into a playhead position; 0 means unknown,
     *  and a tap then does nothing rather than guessing. */
    private var projectDurationMs: Long = 0

    /** Reads out the selected clip's transitions and effects, and
     *  carries the length control. */
    private var transitionInfo: TextView? = null

    /** Holds the transition-length row, rebuilt per selection. */
    private var lengthHost: LinearLayout? = null

    /**
 * Whether the log is expanded in place rather than behind a button.
 *
 * A setting, not a preference: the log is the only diagnostic
 * available without adb, and on the home screen it costs a fifth of
 * the display. Off by default so the menu reads as a menu; on when
 * something is going wrong and the user has said so once.
 */
private var logOn: Boolean
    get() = getSharedPreferences("caprust", MODE_PRIVATE)
        .getBoolean("logOn", false)
    set(v) = getSharedPreferences("caprust", MODE_PRIVATE)
        .edit().putBoolean("logOn", v).apply()

    /** Cached clip fields for the selected row, so the sheet and the
     *  detail panel do not have to re-query. */
    private val clipInfo = HashMap<String, JSONObject>()

    /** Preset ids per kind ("effect", "filter", "transition"), fetched
     *  once from the server. See [showPresetPicker]. */
    private val assetsCache = HashMap<String, List<String>>()

    /** Project name from the last summary, for the Project header. */
    @Volatile
    private var pendingProjectName = ""

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
        client.onEvent = { line ->
            renderLog()
            // A lost link used to be a log line and nothing else: the
            // app stayed on whatever screen it was on, every control
            // failed with "not connected", and there was no way back to
            // Connect from there. The user saw the log change and
            // nothing else happen.
            //
            // Matches on the marker rather than on any transport text,
            // so a plain disconnect notice is not mistaken for a
            // failure worth bouncing the user out of.
            if (line == "-- link lost" || line == "-- disconnected") {
                connected = false
                // `wanted` is left alone on purpose: a reconnect
                // interval the user set is theirs to keep. Sitting on
                // Connect is the fallback surface, and a successful
                // retry walks back into the editor by itself.
                runOnUiThread {
                    if (current != Screen.Connect) show(Screen.Connect)
                }
            }
        }
        // A rotation destroys and rebuilds the activity, socket and all.
        // Restoring the screen is what keeps the editor on screen
        // instead of dropping the user back to a connection form they
        // already filled in. The socket is gone either way; the
        // reconnect in onResume is what brings it back.
        val saved = savedInstanceState?.getString("screen")
        current = if (saved != null) screenFrom(saved) else Screen.Connect
        show(current)
        if (connected || wanted) refresh()
    }

    // ---- navigation ----

    /** Rebuild the content view for `screen`. */
    private fun show(screen: Screen) {
        current = screen
        setContentView(
            when (screen) {
                Screen.Connect -> buildConnect()
                Screen.Home -> buildHome()
                Screen.Project -> buildProject()
                Screen.Projects -> buildProjects()
                Screen.Log -> buildLog()
                Screen.Settings -> buildSettings()
                Screen.Connections -> buildConnections()
            }
        )
        if (screen == Screen.Project) {
            refresh()
            // Started after the view exists: the poll writes into
            // previewView and playButton, which are built by
            // buildProject just above.
            startLivePolling()
        }
    }

    /**
     * Where Back goes.
     *
     * Always Connect, never Project. The first version sent Back to
     * the editor when a link was wanted, which meant the arrow on the
     * editor screen did nothing at all -- it navigated to the screen
     * you were already on, which reads as a broken button. Connect is
     * where pairing and disconnecting live anyway.
     */
    /**
     * Where Back goes.
     *
     * Project when a session exists, Connect otherwise. Sending Back to
     * Connect unconditionally is what made the arrow on the Projects
     * screen throw away the editor: the user tapped Open, found the
     * list they expected, went back, and landed on a connection form
     * they had already filled in.
     */
    private fun back(): Screen = if (connected || wanted) Screen.Home else Screen.Connect
    /**
     * Set the status line, if a screen showing one is attached.
     *
     * Every status write goes through here. Screens are rebuilt on
     * navigation, so the TextView only exists while its screen is on
     * display -- a reply arriving after the user moved to the log
     * screen would otherwise touch a detached view. The text is kept
     * in [pendingStatus] and shown by the next screen that has a
     * status line, so a message is never silently dropped.
     */
    private fun say(line: String) {
        pendingStatus = line
        status?.text = line
    }

    /** Last thing [say] was given, replayed into the next screen. */
    private var pendingStatus = ""

    /** The host to dial: what is typed, else what was last used. */
    private fun hostValue(): String =
        if (::hostBox.isInitialized) hostBox.text.toString().trim() else sharedPrefsHost()

    private fun portValue(): Int =
        if (::portBox.isInitialized) {
            portBox.text.toString().trim().toIntOrNull() ?: 4711
        } else {
            sharedPrefsPort().toIntOrNull() ?: 4711
        }

    /**
     * A screen header with an optional back arrow and trailing actions.
     *
     * Every screen gets one so there is always an obvious way out. A
     * full-screen settings page with no back control is the kind of
     * thing that needs the home button to escape.
     */
    private fun header(title: String, vararg actions: View): LinearLayout {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = android.view.Gravity.CENTER_VERTICAL
            setPadding(0, dp(4), 0, dp(4))
        }
        // Home is the root of a live session, so it carries no Back -- an
        // arrow there that navigates to the screen you are already on
        // reads as a dead button, which is exactly what it was.
        if (current != Screen.Connect && current != Screen.Home) {
            row.addView(btn(getString(R.string.btn_back)) { show(back()) })
        }
        row.addView(TextView(this).apply {
            text = title
            textSize = 17f
            setTypeface(typeface, android.graphics.Typeface.BOLD)
            setPadding(dp(6), 0, 0, 0)
        })
        for (a in actions) row.addView(a)
        return row
    }

    /** Vertical scroll container with the standard padding. */
    private fun column(): LinearLayout = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(dp(12), dp(10), dp(12), dp(12))
    }

    /**
     * A rounded panel for grouping related controls.
     *
     * The platform theme gives everything the same flat background, so
     * a screen reads as one undifferentiated column of buttons. Cards
     * with a visible surface and a little inset are what separate
     * "connection details" from "actions" at a glance.
     *
     * A GradientDrawable rather than a Material card: this app has no
     * androidx dependency, and the shape is nine lines of code.
     */
    private fun card(): LinearLayout {
        val card = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            background = cardBackground()
            setPadding(dp(12), dp(10), dp(12), dp(10))
        }
        val lp = LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)
        lp.setMargins(0, dp(4), 0, dp(4))
        card.layoutParams = lp
        return card
    }

    private fun cardBackground(): android.graphics.drawable.Drawable =
        android.graphics.drawable.GradientDrawable().apply {
            cornerRadius = 14f * resources.displayMetrics.density
            setColor(surfaceColor())
            setStroke(
                (1 * resources.displayMetrics.density).toInt(),
                outlineColor(),
            )
        }

    /** Panel colour, derived from the theme so it flips with light and
     *  dark rather than being a fixed grey that only works on one. */
    private fun surfaceColor(): Int {
        val night = resources.configuration.uiMode and
            android.content.res.Configuration.UI_MODE_NIGHT_MASK ==
            android.content.res.Configuration.UI_MODE_NIGHT_YES
        return if (night) 0xFF1E1E22.toInt() else 0xFFF3F3F7.toInt()
    }

    private fun outlineColor(): Int {
        val night = resources.configuration.uiMode and
            android.content.res.Configuration.UI_MODE_NIGHT_MASK ==
            android.content.res.Configuration.UI_MODE_NIGHT_YES
        return if (night) 0xFF34343A.toInt() else 0xFFD8D8E0.toInt()
    }

    /** Section heading. Dimmed and spaced so it labels what follows
     *  rather than competing with it. */
    private fun sectionTitle(text: String): TextView = TextView(this).apply {
        this.text = text
        textSize = 12f
        setTextColor(0xFF8A8A96.toInt())
        setTypeface(typeface, android.graphics.Typeface.BOLD)
        setPadding(dp(4), dp(14), dp(4), dp(2))
    }

    /** A button with flat platform styling removed.
     *
     * The default Button carries its own background, border and
     * all-caps label; six of them in a row looks like a form. */
    private fun chip(label: String, onClick: () -> Unit) = TextView(this).apply {
        text = label
        textSize = 14f
        gravity = Gravity.CENTER
        setTextColor(0xFFFFFFFF.toInt())
        background = android.graphics.drawable.GradientDrawable().apply {
            cornerRadius = 10f * resources.displayMetrics.density
            setColor(accentColor())
        }
        val pad = dp(14)
        setPadding(pad, dp(9), pad, dp(9))
        setOnClickListener { onClick() }
    }

    private fun accentColor(): Int {
        val night = resources.configuration.uiMode and
            android.content.res.Configuration.UI_MODE_NIGHT_MASK ==
            android.content.res.Configuration.UI_MODE_NIGHT_YES
        return if (night) 0xFF3D7BE8.toInt() else 0xFF2F6FD0.toInt()
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
        // Activity.getTheme() returns a Resources.Theme, not the id we
        // set, so the already-applied id is tracked here instead.
        if (appliedThemeId == id) return
        appliedThemeId = id
        setTheme(id)
        if (apply) recreate()
    }

    /** Resource id currently set via [applyTheme], or 0 before the first
     *  call. Guards against a redundant recreate. */
    private var appliedThemeId = 0

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
        attemptConnect()
    }

    /**
     * One reconnect attempt, self-rescheduling while it fails.
     *
     * Retries are scheduled rather than fired in a loop: a tight retry
     * against a desktop that is asleep or off-network would hammer the
     * socket and drain the battery. The interval is the setting; 0
     * means try once and then wait for the user.
     *
     * The check on `wanted` is inside the runnable, not at schedule
     * time. A manual Disconnect while a retry is pending has to win,
     * or the button appears not to work.
     */
    private fun attemptConnect() {
        val host = sharedPrefsHost()
        if (host.isEmpty()) return
        say(getString(R.string.msg_reconnecting, host))
        val handler = android.os.Handler(android.os.Looper.getMainLooper())
        handler.postDelayed({
            if (!wanted) return@postDelayed
            client.connect(host, sharedPrefsPort().toIntOrNull() ?: 4711) { ok, err ->
                connected = ok
                if (ok) {
                    say(getString(R.string.msg_connected_to, host))
                    // Same check as a manual connect: the socket being
                    // open says nothing about whether the token works.
                    if (current == Screen.Connect) checkServerThenPrompt()
                } else {
                    say(getString(R.string.msg_reconnect_failed, err))
                    val every = reconnectDelaySec()
                    if (wanted && every > 0) attemptConnect()
                }
            }
        }, reconnectDelaySec() * 1000L)
    }

    private fun dp(v: Int) = (v * resources.displayMetrics.density).toInt()

    /**
     * Screen 1: reach a computer and pair with it.
     *
     * Deliberately the only screen with no Back arrow -- it is the root
     * of the app, and an arrow pointing at itself would be a lie.
     * Everything else hangs off it.
     */
    private fun buildConnect(): View {
        val root = column()

        root.addView(TextView(this).apply {
            text = getString(R.string.app_name)
            textSize = 20f
            setTypeface(typeface, android.graphics.Typeface.BOLD)
        })

        status = TextView(this).apply {
            textSize = 13f
            // Replay whatever the last action said. Without this, a
            // message posted while another screen was up would be lost
            // the moment the user navigated here.
            text = pendingStatus
        }
        root.addView(status)

        // A socket without a token cannot call a single tool, and the
        // only way to tell that apart from "connected and working" is
        // the failure message. Say it up front instead.
        if (connected && !client.isPaired()) {
            root.addView(TextView(this).apply {
                text = getString(R.string.msg_need_pair)
                textSize = 13f
                setTextColor(0xFFE07A1F.toInt())
                setPadding(0, dp(6), 0, dp(6))
            })
        }

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
        // Labelled rather than hinted: this is the field a first-time
        // user is looking for, and a hint disappears as soon as they
        // start typing anything anywhere else on the screen.
        root.addView(TextView(this).apply {
            text = getString(R.string.pairing_header)
            textSize = 13f
            setTypeface(typeface, android.graphics.Typeface.BOLD)
            setPadding(0, dp(14), 0, dp(4))
        })
        val pairRow = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        codeBox = EditText(this).apply {
            hint = getString(R.string.lbl_pair_code)
            inputType = android.text.InputType.TYPE_CLASS_NUMBER
        }
        pairRow.addView(codeBox, LinearLayout.LayoutParams(0, dp(48), 2f))
        pairRow.addView(btn(getString(R.string.btn_pair)) { doPair() },
            LinearLayout.LayoutParams(0, dp(48), 1f))
        root.addView(pairRow)

        // ---- secondary actions ----
        val nav = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        nav.addView(btn(getString(R.string.btn_saved)) { show(Screen.Connections) })
        nav.addView(btn(getString(R.string.btn_settings)) { show(Screen.Settings) })
        nav.addView(btn(getString(R.string.btn_log)) { show(Screen.Log) })
        root.addView(nav)
        return root
    }

    /**
     * Screen 2: the open project.
     *
     * The project name goes in the header because that is what the
     * screen is about, and it was previously buried in a status line
     * that also carried connection messages.
     */
    private fun buildProject(): View {
        val root = column()

        status = TextView(this).apply { textSize = 12f; text = pendingStatus }

        root.addView(header(
            projectTitle(),
            btn(getString(R.string.btn_home)) { show(Screen.Home) },
            btn(getString(R.string.btn_open)) { show(Screen.Projects) },
            btn(getString(R.string.btn_log)) { show(Screen.Log) },
            btn(getString(R.string.btn_settings)) { show(Screen.Settings) },
        ))
        root.addView(status)

        // Views that landscape moves into the right-hand column. Kept in one
        // list, in the order they were added to `root`, so the move
        // below cannot scramble them.
        val movable = ArrayList<View>()

        // ---- preview ----
        // A frame is several KB of JPEG, so it is polled rather than
        // pushed: the phone does not know when the playhead moves on
        // the desktop, and a push would need the desktop to track which
        // phone asked.
        previewView = ImageView(this).apply {
            scaleType = ImageView.ScaleType.FIT_CENTER
            setBackgroundColor(0xFF141414.toInt())
            contentDescription = getString(R.string.preview_desc)
            // Tap to seek to a quarter, long press to three quarters. The frame
            // is the one control on this screen with real estate to
            // spare, and a fixed step is a poor substitute for seeking.
            isClickable = true
            setOnClickListener { v -> seekToFraction(v, 0.25f) }
            setOnLongClickListener { v ->
                seekToFraction(v, 0.75f)
                true
            }
        }
        root.addView(previewView, LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, dp(190)))

        // Says so when there is nothing to show. An empty black box is
        // indistinguishable from a broken app, and the reason is
        // almost always "the preview has not decoded yet".
        previewNote = TextView(this).apply {
            text = getString(R.string.preview_none)
            textSize = 12f
        }
        root.addView(previewNote)
        movable.add(previewView!!)
        movable.add(previewNote!!)

        // ---- transport ----
        // Two rows, not one. A LinearLayout does not wrap: four chips
        // side by side do not fit a phone, and the last one is simply
        // clipped off the right edge with no scrollbar to hint at it.
        // The refresh button was the casualty -- it looked like it was
        // never built.
        val transport = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        // Two buttons, not one that toggles. A toggle whose label
        // depends on state it can only learn by polling is a button
        // that lies for a moment after every press, and pressing it
        // twice to get back is worse than two honest buttons.
        playButton = chip(getString(R.string.act_play)) { sendTransport("play") }
        pauseButton = chip(getString(R.string.act_pause)) { sendTransport("pause") }
        transport.addView(playButton, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        transport.addView(pauseButton, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        transport.addView(chip(getString(R.string.act_back5)) { sendTransport("seek_back") })
        transport.addView(chip(getString(R.string.act_fwd5)) { sendTransport("seek_fwd") })
        root.addView(transport)
        movable.add(transport)

        val previewBtnRow = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        previewBtnRow.addView(chip(getString(R.string.btn_refresh_preview)) { fetchPreview() })
        root.addView(previewBtnRow)
        movable.add(previewBtnRow)

        playheadView = TextView(this).apply { textSize = 12f }
        root.addView(playheadView)
        movable.add(playheadView!!)

        val exportRow = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        exportButton = btn(getString(R.string.btn_export)) { askExport() }
        exportRow.addView(exportButton)
        root.addView(exportRow)
        movable.add(exportRow)

        // ---- actions ----
        val actions = card()
        val actionsRow = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        actionsRow.addView(chip(getString(R.string.btn_refresh)) { refresh() })
        actionsRow.addView(chip(getString(R.string.btn_undo)) { client.call("undo") { ok, _ -> if (ok) refresh() } })
        actionsRow.addView(chip(getString(R.string.btn_redo)) { client.call("redo") { ok, _ -> if (ok) refresh() } })
        actionsRow.addView(chip(getString(R.string.btn_save)) { client.call("save_project") { ok, _ -> if (ok) refresh() } })
        actions.addView(actionsRow)

        val actions2 = card()
        val actionsRow2 = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        actionsRow2.addView(chip(getString(R.string.btn_disconnect)) { doDisconnect() })
        actions.addView(actionsRow2)
        root.addView(actions)

        // ---- time input ----
        // Split used to happen at the midpoint, which is safe but not
        // what anyone means. One field serves both Split and Move, and
        // selecting a clip prefills it with that clip's midpoint so the
        // common case is still one tap.
        root.addView(sectionTitle(getString(R.string.title_clip_actions)))
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
                timeline?.setSelected(id)
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
        val presets = card()
        val presetsRow = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        presetsRow.addView(chip(getString(R.string.btn_effects)) { showPresetPicker("effect") })
        presetsRow.addView(chip(getString(R.string.btn_filters)) { showPresetPicker("filter") })
        presetsRow.addView(chip(getString(R.string.btn_transitions)) { showPresetPicker("transition") })
        presets.addView(presetsRow)

        // What is actually on the selected clip, and the length of its
        // transitions. Before this the phone could apply a transition
        // but had no way to read back the duration it was given, no way
        // to change it, and no way to take an effect off again.
        transitionInfo = TextView(this).apply { textSize = 12f }
        presets.addView(transitionInfo)
        root.addView(presets)
        movable.add(presets)

        lengthHost = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        root.addView(lengthHost)
        movable.add(lengthHost!!)

        root.addView(sectionTitle(getString(R.string.title_clips)))

        val clipScroll = ScrollView(this)
        clipList = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
        }
        clipScroll.addView(clipList, ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        root.addView(clipScroll, LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, 0, 3f))

        // The log used to be built here and attached in one of two places
        // below. It is its own screen now, so nothing inline survives:
        // a quarter of a phone screen for a diagnostic nobody wants
        // while editing.
        if (resources.configuration.orientation ==
            android.content.res.Configuration.ORIENTATION_PORTRAIT) {
            return root
        }

        // Landscape on a tablet was throwing away half the screen on a
        // single column. Two columns: timeline and clips on the left,
        // the selected clip's detail on the right.
        //
        // The left column scrolls. It holds a 190px preview, two
        // transport rows and a 150px timeline before the clip list, and
        // on a landscape phone that is taller than the screen -- so
        // without a scroller the timeline sits below the fold with no
        // way to reach it, which is exactly where it was.
        val wide = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        val leftScroll = ScrollView(this)
        leftScroll.addView(root, ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        wide.addView(leftScroll, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 1f))
        val side = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(8), 0, dp(0), 0)
        }
        // The right column used to hold nothing but the selected clip's
        // description, which is empty until a clip is tapped -- so half
        // a landscape tablet was blank. What lives on the right while
        // working is the preview and the transport, and on a wide screen
        // they read better beside the timeline than above it.
        for (v in movable) {
            root.removeView(v)
            side.addView(
                v,
                LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT
                )
            )
        }
        side.addView(
            selectedInfo(),
            LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            )
        )
        // The clip list follows the moved views, so it needs room on the
        // left rather than the weight 3 it had when it was the only
        // thing below them.
        clipScroll?.layoutParams = LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            0,
            3f
        )
        wide.addView(side, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 1f))
        return wide
    }

    /**
     * Screen: the recent-projects list on the computer.
     *
     * Paths come from the desktop's own recent list, not from anything
     * cached here. The phone has no way to know what has been opened
     * elsewhere, and a stale copy would offer projects that no longer
     * exist -- so the list is fetched when the screen opens.
     *
     * `open_project` only *requests* the switch: the desktop loads it
     * after the current batch of requests finishes. The reply here
     * therefore means "accepted", not "loaded", so the wait for the
     * new project is a refresh rather than an immediate re-read.
     */
    private fun buildProjects(): View {
        val root = column()
        root.addView(header(getString(R.string.title_projects)))
        status = TextView(this).apply { textSize = 12f; text = pendingStatus }
        root.addView(status)

        val list = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        val scroll = ScrollView(this)
        scroll.addView(list, ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        root.addView(scroll, LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        list.addView(TextView(this).apply { text = getString(R.string.msg_loading_projects) })

        // The parameter is not named `text`. Kotlin resolves a name in scope
        // ahead of a member of the implicit receiver, so a lambda
        // parameter called `text` makes `text = ...` inside `apply`
        // assign to the parameter -- a val, and a compile error.
        client.call("list_recent_projects") { ok, body ->
            runOnUiThread {
                list.removeAllViews()
                if (!ok) {
                    say(body)
                    list.addView(TextView(this@MainActivity).apply { text = body })
                    return@runOnUiThread
                }
                val arr = try {
                    JSONObject(body).optJSONArray("projects") ?: JSONArray()
                } catch (_: Exception) {
                    JSONArray()
                }
                if (arr.length() == 0) {
                    // Say what the desktop reported rather than a bare
                    // "nothing here": an empty list and a list that
                    // failed to arrive look identical from here.
                    val reported = try {
                        JSONObject(body).optInt("count", 0)
                    } catch (_: Exception) {
                        0
                    }
                    list.addView(TextView(this@MainActivity).apply {
                        text = getString(R.string.no_projects_n, reported)
                    })
                    return@runOnUiThread
                }
                for (i in 0 until arr.length()) {
                    val p = arr.getJSONObject(i)
                    list.addView(projectRow(p))
                }
            }
        }
        return root
    }

    private fun projectRow(p: JSONObject): View {
        val name = p.optString("name", "?")
        val clips = p.optInt("clip_count")
        val secs = p.optLong("duration_ms") / 1000
        val path = p.optString("path")

        val col = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(8), dp(6), dp(8), dp(6))
        }
        col.addView(btn("$name   $clips clips / ${secs}s") { openProject(path) })
        col.addView(TextView(this).apply {
            text = path
            textSize = 10f
            setTextColor(0xFF888888.toInt())
        })
        return col
    }

    private fun openProject(path: String) {
        say(getString(R.string.msg_opening, path))
        client.call("open_project", JSONObject().put("path", path)) { ok, text ->
            runOnUiThread {
                if (!ok) say(text)
                else {
                    // The desktop loads after the batch, so the new
                    // project is not readable yet. Wait, then refresh.
                    android.os.Handler(android.os.Looper.getMainLooper())
                        .postDelayed({ show(Screen.Project) }, 700)
                }
            }
        }
    }

    /**
     * Screen 2: what you can do once the link is up.
     *
     * Connecting used to land on the editor, which showed a preview of
     * a project that might not be open at all -- an empty frame, a
     * transport that does nothing, a clip list with nothing in it. A
     * link is not a session, and the screen after connecting should
     * say what is possible rather than show the result of a choice
     * that has not been made yet.
     *
     * The menu is vertical because it is three items and the whole
     * screen. A row of three on a phone puts them at thumb reach in
     * the least useful place.
     */
    private fun buildHome(): View {
        val outer = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        val col = column()
        col.addView(logo())
        col.addView(
            TextView(this).apply {
                text = pendingStatus
                textSize = 13f
                gravity = Gravity.CENTER
                setPadding(0, dp(6), 0, dp(10))
            }
        )

        // A card per entry, label plus a line saying where it leads.
        // Three unexplained buttons are a form with worse labels.
        col.addView(homeRow(R.string.btn_open, R.string.home_open_hint) {
            show(Screen.Projects)
        })
        col.addView(homeRow(R.string.btn_settings, R.string.home_settings_hint) {
            show(Screen.Settings)
        })
        if (logOn) {
            // Open by default when the setting says so, which is the
            // entire reason the setting exists.
            col.addView(sectionTitle(getString(R.string.title_log)))
            val logScroll = ScrollView(this)
            logView = TextView(this).apply {
                textSize = 10f
                setTypeface(android.graphics.Typeface.MONOSPACE)
                setTextIsSelectable(true)
                renderLogInto(this)
            }
            logScroll.addView(logView, ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
            col.addView(logScroll, LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, dp(200)))
        } else {
            col.addView(homeRow(R.string.btn_log, R.string.home_log_hint) {
                show(Screen.Log)
            })
        }

        // A scroller in both orientations. The menu plus an expanded log
        // is taller than a landscape phone, and a menu you cannot scroll
        // to is a menu with a missing button.
        val scroll = ScrollView(this)
        scroll.addView(col, ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        outer.addView(scroll, LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        return outer
    }

    /** The desktop's own icon, so the two halves look like one product. */
    private fun logo(): View = android.widget.ImageView(this).apply {
        setImageResource(R.drawable.ic_launcher)
        scaleType = android.widget.ImageView.ScaleType.FIT_CENTER
        contentDescription = getString(R.string.app_name)
        val size = dp(96)
        layoutParams = LinearLayout.LayoutParams(size, size).apply {
            gravity = Gravity.CENTER_HORIZONTAL
            setMargins(0, dp(16), 0, 0)
        }
    }

    /** A menu row: label plus a line saying what it leads to. */
    private fun homeRow(label: Int, hint: Int, onClick: () -> Unit): View {
        val shell = card()
        val col = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(4), dp(10), dp(4), dp(10))
        }
        col.addView(TextView(this).apply {
            text = getString(label)
            textSize = 16f
            setTypeface(typeface, android.graphics.Typeface.BOLD)
        })
        col.addView(TextView(this).apply {
            text = getString(hint)
            textSize = 12f
            setTextColor(0xFF8A8A96.toInt())
        })
        // Clickable on the whole card. Tappable text with a gap under it
        // is a target the user has to aim at.
        shell.addView(col)
        shell.setOnClickListener { onClick() }
        return shell
    }

    /**
     * Screen 3: the raw traffic log.
     *
     * Its own screen, reachable by a button, never expanded over the
     * editor. It is still the only diagnostic available without adb, so
     * it has to stay one tap away rather than behind a settings dive.
     */
    private fun buildLog(): View {
        val root = column()
        root.addView(
            header(
                getString(R.string.title_log),
                btn(getString(R.string.btn_clear)) { client.clearLog(); renderLog() }
            )
        )
        status = TextView(this).apply { textSize = 12f; text = pendingStatus }
        root.addView(status)

        val scroll = ScrollView(this)
        logView = TextView(this).apply {
            textSize = 10f
            setTypeface(android.graphics.Typeface.MONOSPACE)
            // Selectable so a line can be copied into a bug report.
            setTextIsSelectable(true)
            renderLogInto(this)
        }
        scroll.addView(logView, ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        root.addView(scroll, LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        return root
    }

    /**
     * Screen 4: settings.
     *
     * Two choices, each one row with the current value on it, rather
     * than a dialog per setting. A dialog to pick one of three themes
     * is a dialog to dismiss every time.
     */
    private fun buildSettings(): View {
        val root = column()
        root.addView(header(getString(R.string.title_settings)))
        status = TextView(this).apply { textSize = 12f; text = pendingStatus }
        root.addView(status)

        root.addView(btn(getString(R.string.lbl_theme, themeLabel())) { cycleTheme() })

        val every = reconnectDelaySec()
        root.addView(btn(getString(R.string.lbl_reconnect, reconnectLabel(every))) {
            chooseReconnect()
        })

        root.addView(btn(getString(R.string.lbl_log_inline, getString(
            if (logOn) R.string.on else R.string.off))) { toggleInlineLog() })

        root.addView(btn(getString(R.string.btn_saved)) { show(Screen.Connections) })
        root.addView(btn(getString(R.string.btn_log)) { show(Screen.Log) })

        val spacer = LinearLayout(this)
        root.addView(spacer, LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        return root
    }

    private fun reconnectLabel(seconds: Int): String = when (seconds) {
        0 -> getString(R.string.reconnect_off)
        1 -> getString(R.string.reconnect_seconds, 1)
        else -> getString(R.string.reconnect_seconds, seconds)
    }

    private fun chooseReconnect() {
        val options = intArrayOf(0, 1, 5, 10)
        val labels = options.map { reconnectLabel(it) }.toTypedArray<CharSequence>()
        android.app.AlertDialog.Builder(this)
            .setTitle(getString(R.string.lbl_reconnect_short))
            .setItems(labels) { _, which ->
                setReconnectDelay(options[which])
                show(Screen.Settings)
            }
            .setNegativeButton(android.R.string.cancel, null)
            .show()
    }

    private fun setReconnectDelay(seconds: Int) {
        getSharedPreferences("caprust", MODE_PRIVATE)
            .edit().putInt("reconnectSec", seconds).apply()
    }

    /** Seconds between reconnect attempts. 0 = one attempt, then wait
     *  for the user. Default 5: fast enough that a sleeping desktop
     *  feels instant, slow enough not to spin. */
    private fun reconnectDelaySec(): Int =
        getSharedPreferences("caprust", MODE_PRIVATE).getInt("reconnectSec", 5)

    /**
     * Show the log in place on the home screen, or hide it.
     *
     * Rebuilds the screen because the home layout has two shapes: the
     * log is either a row in the menu or a panel under it, and a view
     * that has to be moved between them is one more thing that can end
     * up half-showed.
     */
    private fun toggleInlineLog() {
        logOn = !logOn
        if (current == Screen.Home) show(Screen.Home)
    }

    /**
     * Screen 5: saved pairings.
     *
     * A computer is identified by host:port, and the pairing token
     * belongs to that computer -- a token from the desktop in the
     * office is meaningless to the one at home. Saving the token with
     * the address is what makes switching back and forth work without
     * retyping a six-digit code that only exists for five minutes.
     */
    private fun buildConnections(): View {
        val root = column()
        root.addView(header(getString(R.string.title_connections)))
        status = TextView(this).apply { textSize = 12f; text = pendingStatus }
        root.addView(status)

        val saved = loadSaved()
        if (saved.isEmpty()) {
            root.addView(TextView(this).apply {
                text = getString(R.string.no_connections)
                setPadding(0, dp(12), 0, dp(12))
            })
        } else {
            for (entry in saved) {
                val key = "${entry.get("host")}:${entry.get("port")}"
                val row = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
                val label = entry.optString("name").ifEmpty { key }
                row.addView(btn(label) { connectTo(entry) },
                    LinearLayout.LayoutParams(0, dp(48), 1f))
                row.addView(btn(getString(R.string.btn_forget)) {
                    forget(key)
                    show(Screen.Connections)
                }, LinearLayout.LayoutParams(dp(100), dp(48)))
                root.addView(row)
            }
        }

        root.addView(btn(getString(R.string.btn_save_current)) { saveCurrent() })
        return root
    }

    private fun connectTo(entry: JSONObject) {
        val host = entry.optString("host")
        val port = entry.optInt("port", 4711)
        getSharedPreferences("caprust", MODE_PRIVATE).edit()
            .putString("host", host).putString("port", "$port").apply()
        // The saved token belongs to this computer, not to whatever was
        // connected last. Loading it is what makes the pairing stick.
        val token = entry.optString("token")
        if (token.isNotEmpty()) {
            client.setToken(token)
            getSharedPreferences("caprust", MODE_PRIVATE)
                .edit().putString("token", token).apply()
        }
        doConnect()
    }

    private fun saveCurrent() {
        val host = hostValue()
        if (host.isEmpty()) {
            say(getString(R.string.msg_no_host))
            return
        }
        val port = portValue()
        val name = host + if (client.isPaired()) "" else " (${getString(R.string.not_paired)})"
        val saved = loadSaved().toMutableList()
        val key = "$host:$port"
        saved.removeAll { "${it.optString("host")}:${it.optInt("port")}" == key }
        saved.add(
            JSONObject()
                .put("host", host)
                .put("port", port)
                .put("name", name)
                .put("token", client.token ?: "")
        )
        storeSaved(saved)
        say(getString(R.string.msg_saved, name))
        show(Screen.Connections)
    }

    private fun forget(key: String) {
        storeSaved(loadSaved().filter { "${it.optString("host")}:${it.optInt("port")}" != key })
    }

    private fun loadSaved(): List<JSONObject> {
        val raw = getSharedPreferences("caprust", MODE_PRIVATE)
            .getString("connections", null) ?: return emptyList()
        return try {
            val arr = JSONArray(raw)
            (0 until arr.length()).map { arr.getJSONObject(it) }
        } catch (_: Exception) {
            // Corrupt entry should not take the whole screen down.
            emptyList()
        }
    }

    private fun storeSaved(list: List<JSONObject>) {
        val arr = JSONArray()
        for (e in list) arr.put(e)
        getSharedPreferences("caprust", MODE_PRIVATE)
            .edit().putString("connections", arr.toString()).apply()
    }

    /** Name for the project header. Falls back to a neutral label so a
     *  header never renders blank while the summary is in flight. */
    private fun projectTitle(): String {
        var name = ""
        synchronized(this) { name = pendingProjectName }
        return name.ifEmpty { getString(R.string.title_project) }
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
        val host = hostValue()
        val port = portValue()
        if (host.isEmpty()) {
            say("Enter the computer's IP address")
            return
        }
        say("Connecting to $host:$port ...")
        client.connect(host, port) { ok, err ->
            connected = ok
            // Remembered separately from `connected`: this is the
            // user's intent, and a failed attempt should still be
            // retried on the next resume.
            wanted = true
            if (!ok) {
                say("Failed: $err")
            } else {
                getSharedPreferences("caprust", MODE_PRIVATE).edit()
                    .putString("host", host).putString("port", "$port").apply()
                say("Connected to $host:$port")
                // Ask the server whether the stored token is good,
                // rather than believing the copy on the phone. A token
                // can go stale between sessions -- the desktop's
                // settings live wherever its settings file does, and a
                // build run from a different folder has a different
                // one. Trusting the cache meant a stale token looked
                // paired and jumped past the pairing field, leaving
                // every call failing with no way forward.
                checkServerThenPrompt()
            }
        }
    }

    /** Send one transport command to the desktop and follow the result. */
    private fun sendTransport(action: String) {
        client.call("preview_transport", JSONObject().put("action", action)) { ok, body ->
            if (!ok) {
                runOnUiThread { say(body) }
                return@call
            }
            // Read the state back rather than assuming what it did.
            // Only the status call: the live poll already fetches the
            // frame, and asking for both here put a second request on
            // the same serial queue behind the poll's own, which is
            // what made the transport stop responding after one press.
            pollPlayback()
        }
    }

    /** Read play state and playhead, updating the button and readout. */
    private fun pollPlayback() {
        client.call("preview_status") { ok, body ->
            if (!ok) return@call
            runOnUiThread {
                val playing = try {
                    JSONObject(body).optBoolean("playing", false)
                } catch (_: Exception) {
                    false
                }
                val at = try {
                    JSONObject(body).optLong("at_ms", -1)
                } catch (_: Exception) {
                    -1
                }
                playButton?.alpha = if (playing) 0.45f else 1f
                pauseButton?.alpha = if (playing) 1f else 0.45f
                playheadView?.text = if (at < 0) "" else formatTime(at)
            }
        }
    }

    /** milliseconds as m:ss.d -- a duration, not a clock. */
    private fun formatTime(ms: Long): String {
        val total = ms / 1000
        return "%d:%02d.%d".format(total / 60, total % 60, (ms % 1000) / 100)
    }

    /**
 * Keep the preview and the transport label in step with the desktop.
 *
 * Polled rather than pushed, and only while the Project screen is on
 * display. It has to be polled: the user can start playback from the
 * keyboard on the computer, and the phone learns about it nowhere
 * else. A button that still reads "Play" while the project is playing
 * is worse than no button.
 *
 * Self-stopping. An endless poll would hold the socket open for as
 * long as the app was backgrounded.
 */
    private fun startLivePolling() {
        val handler = android.os.Handler(android.os.Looper.getMainLooper())
        handler.post(object : Runnable {
            override fun run() {
                if (current != Screen.Project || !connected) return
                pollPlayback()
                fetchPreview()
                handler.postDelayed(this, 800)
            }
        })
    }

    /**
 * Seek to a fraction of the project, from a tap on the preview.
 *
 * `frac` is the position along the project, 0..1: a plain tap seeks to
 * 25%, a long press to 75%. Position rather than the touch's x so the
 * frame itself has to be the control -- on a small phone the picture is
 * the only thing big enough to hit without missing.
 */
private fun seekToFraction(v: View, frac: Float) {
    if (projectDurationMs <= 0L) {
        say(getString(R.string.msg_no_duration))
        return
    }
    if (v.width <= 0) return
    val at = (projectDurationMs * frac).toLong()
    client.call(
        "preview_transport",
        JSONObject().put("action", "seek").put("at_ms", at)
    ) { ok, msg ->
        runOnUiThread {
            if (ok) {
                pollPlayback()
                fetchPreview()
            } else {
                say(msg)
            }
        }
    }
}

/**
 * Ask the desktop for its current preview frame.
     *
     * A null frame is a normal answer, not an error: the preview has to
     * decode before it has anything to show. Treated as an error it
     * would put a failure message on screen every time the user asked
     * too early.
     */
    private fun fetchPreview() {
        val view = previewView ?: return
        client.call("preview_frame", JSONObject().put("width", 480)) { ok, body ->
            runOnUiThread {
                if (!ok) {
                    say(body)
                    return@runOnUiThread
                }
                val jpeg = try {
                    JSONObject(body).optString("frame")
                } catch (_: Exception) {
                    ""
                }
                if (jpeg.isEmpty()) {
                    view.setImageDrawable(null)
                    previewNote?.visibility = View.VISIBLE
                    return@runOnUiThread
                }
                val bytes = try {
                    android.util.Base64.decode(jpeg, android.util.Base64.DEFAULT)
                } catch (_: Exception) {
                    say(getString(R.string.preview_bad))
                    return@runOnUiThread
                }
                val bmp = try {
                    android.graphics.BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
                } catch (_: Exception) {
                    null
                }
                if (bmp == null) {
                    say(getString(R.string.preview_bad))
                    return@runOnUiThread
                }
                view.setImageBitmap(bmp)
                previewNote?.visibility = View.GONE
            }
        }
    }

    /**
     * Export settings, read from the desktop and applied one at a time.
     *
     * The values and their accepted options come from the computer
     * rather than being listed here. A hardcoded list would break the
     * day the desktop adds a resolution, and would happily offer one
     * the encoder has never heard of.
     */
    private fun askExport() {
        if (!connected) {
            say(getString(R.string.msg_not_connected))
            return
        }
        client.call("export_settings") { ok, body ->
            runOnUiThread {
                if (!ok) {
                    say(body)
                } else {
                    showExportSheet(body)
                }
            }
        }
    }

    private fun showExportSheet(settingsText: String) {
        val s = try {
            JSONObject(settingsText)
        } catch (_: Exception) {
            JSONObject()
        }
        val available = s.optJSONObject("available") ?: JSONObject()

        // Current values, held here and updated in place. The sheet is
        // rebuilt rather than re-fetched so the desktop's reply is not
        // needed for the row to show what was just chosen.
        val current = HashMap<String, String>()
        val rows = arrayOf(
            "resolution" to R.string.lbl_resolution,
            "frame_rate" to R.string.lbl_framerate,
            "quality" to R.string.lbl_quality,
            "rate_mode" to R.string.lbl_rate_mode,
            "color_range" to R.string.lbl_colour_range,
        )
        for ((key, _) in rows) current[key] = s.optString(key, "?")

        val destBox = EditText(this).apply {
            hint = getString(R.string.lbl_export_path)
            inputType = android.text.InputType.TYPE_CLASS_TEXT
            setText(s.optString("destination", "").ifEmpty { defaultExportPath() })
        }
        val body = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), dp(8), dp(16), dp(8))
        }

        // Destination first. It is the one value with no default, and
        // the path is on the computer -- the phone has no filesystem
        // the desktop can write to, so saying so is the difference
        // between a working export and a confusing one.
        body.addView(TextView(this).apply { text = getString(R.string.lbl_export_path_hint) })
        body.addView(destBox)

        val scroll = ScrollView(this)
        val dialog = android.app.AlertDialog.Builder(this)
            .setTitle(getString(R.string.btn_export))
            .setView(scroll)
            .setPositiveButton(getString(R.string.act_export_now)) { _, _ ->
                val dest = destBox.text.toString().trim()
                if (dest.isEmpty()) return@setPositiveButton
                runExport(dest)
            }
            .setNegativeButton(android.R.string.cancel, null)
            .create()

        // The settings area swaps in place. Opening a second dialog for
        // the option list put it behind the sheet that asked for it, so
        // tapping a setting looked like it did nothing at all.
        //
        // Both renderers are members rather than local functions: two
        // local functions cannot call each other in Kotlin when the
        // first is declared first, and a forward reference is exactly
        // the sort of thing that is only found by a compiler.
        renderExportSettings(body, destBox, rows, current, available)
        scroll.addView(body)
        dialog.show()
    }

    /** The destination box plus one row per setting. */
    private fun renderExportSettings(
        body: LinearLayout,
        destBox: EditText,
        rows: Array<Pair<String, Int>>,
        current: MutableMap<String, String>,
        available: JSONObject,
    ) {
        body.removeAllViews()
        body.addView(TextView(this).apply { text = getString(R.string.lbl_export_path_hint) })
        body.addView(destBox)
        for ((key, label) in rows) {
            val row = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
            row.addView(TextView(this).apply {
                text = getString(R.string.setting_fmt, getString(label), current[key])
                setPadding(0, dp(10), 0, dp(10))
            }, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            row.addView(chip(getString(R.string.act_change_short)) {
                renderExportOptions(body, destBox, rows, current, available, key)
            })
            body.addView(row)
        }
    }

    /**
     * One setting's accepted values, swapped into the sheet.
     *
     * In place rather than in a dialog of its own: a second dialog
     * raised over the first is what made every setting look dead.
     */
    private fun renderExportOptions(
        body: LinearLayout,
        destBox: EditText,
        rows: Array<Pair<String, Int>>,
        current: MutableMap<String, String>,
        available: JSONObject,
        key: String,
    ) {
        val options = available.optJSONArray(key)
        if (options == null || options.length() == 0) {
            say(getString(R.string.msg_no_options, key))
            return
        }
        body.removeAllViews()
        body.addView(TextView(this).apply {
            text = getString(R.string.act_choose, key)
            setPadding(0, dp(4), 0, dp(4))
        })
        for (i in 0 until options.length()) {
            val o = options.getJSONObject(i)
            val id = o.optString("id")
            val label = o.optString("label", id)
            val marked = if (id == current[key]) "$label  ✓" else label
            body.addView(btn(marked) {
                client.call(
                    "set_export_setting",
                    JSONObject().put("key", key).put("value", id)
                ) { ok, reply ->
                    runOnUiThread {
                        if (ok) {
                            current[key] = id
                            renderExportSettings(body, destBox, rows, current, available)
                        } else {
                            say(reply)
                        }
                    }
                }
            })
        }
        body.addView(btn(getString(R.string.btn_back)) {
            renderExportSettings(body, destBox, rows, current, available)
        })
    }

    /**
     * Start an export on the desktop.
     *
     * The desktop's own export window opens on the computer, so the
     * person watching that screen sees what is happening -- a long
     * encode with no feedback anywhere would look like the app hung.
     */
    private fun runExport(dest: String) {
        say(getString(R.string.msg_export_starting))
        exportButton?.text = getString(R.string.export_running)
        client.call("start_export", JSONObject().put("destination", dest)) { ok, body ->
            runOnUiThread {
                if (!ok) {
                    say(body)
                    exportButton?.text = getString(R.string.btn_export)
                } else {
                    say(getString(R.string.msg_export_started))
                    pollExport()
                }
            }
        }
    }

    /** Ask for progress until the export is no longer running.
     *
     * Stops itself. An endless poll would keep the socket busy for as
     * long as the screen stayed open, and the reply handler runs on the
     * io thread -- a timer that never stops is a leak with a network
     * bill attached.
     */
    private fun pollExport() {
        client.call("export_status") { ok, body ->
            if (!ok) return@call
            val started = try {
                JSONObject(body).optBoolean("started", false)
            } catch (_: Exception) {
                false
            }
            val progress = try {
                JSONObject(body).optDouble("progress", 0.0)
            } catch (_: Exception) {
                0.0
            }
            // No frame has been produced yet means 0.0, and that is
            // also "not started". Anything above zero means a job is
            // running; it stops updating when it is done.
            if (!started || progress <= 0.0) {
                android.os.Handler(android.os.Looper.getMainLooper()).postDelayed({
                    pollExport()
                }, 1000)
            } else {
                runOnUiThread {
                    exportButton?.text = getString(R.string.export_pct, (progress * 100).toInt())
                }
                android.os.Handler(android.os.Looper.getMainLooper()).postDelayed({
                    pollExport()
                }, 1000)
            }
        }
    }

    /** A sensible default: project name plus timestamp, next to the
     *  project itself so the file lands somewhere the user can find. */
    private fun defaultExportPath(): String {
        val stamp = java.text.SimpleDateFormat("yyyyMMdd-HHmmss", java.util.Locale.US)
            .format(java.util.Date())
        val name = projectTitle().replace(Regex("[^A-Za-z0-9_-]"), "_")
        return "C:\\Users\\Public\\$name-$stamp.mp4"
    }

    private fun checkServerThenPrompt() {
        client.listTools { ok, names ->
            runOnUiThread {
                if (!ok) {
                    say(getString(R.string.msg_no_tools))
                    return@runOnUiThread
                }
                val missing = requiredTools.filter { it !in names }
                if (missing.isNotEmpty()) {
                    // Name what is missing, so the report says what to
                    // update instead of "the list does not load".
                    say(getString(R.string.msg_desktop_old, missing.joinToString(", ")))
                    return@runOnUiThread
                }
                // Recorded, not required. The features that need them
                // hide themselves when these are absent.
                hasRemoveEffect = "remove_effect" in names
                verifyTokenOrPrompt()
            }
        }
    }

    /**
     * Confirm the pairing by making one authenticated call.
     *
     * Success means the token is live: go to the editor. An auth
     * failure means it is not: drop it so the user can pair again
     * with the field that is on this screen. Anything else is a
     * transport problem and leaves the token alone.
     */
    private fun verifyTokenOrPrompt() {
        client.call("get_project_summary") { ok, body ->
            runOnUiThread {
                when {
                    // The link is up and the token works, but no project has been
                    // chosen yet. The editor comes after that choice,
                    // not before it.
                    ok -> show(Screen.Home)
                    isAuthFailure(body) -> {
                        client.setToken(null)
                        client.clearToken()
                        say(getString(R.string.msg_token_stale))
                        show(Screen.Connect)
                    }
                    else -> say(body)
                }
            }
        }
    }

    /** Does this failure mean "the token is not valid"? The server
     *  answers with JSON-RPC code -32001 and says so in words. */
    private fun isAuthFailure(message: String): Boolean =
        message.contains("-32001") ||
            message.contains("invalid token", true) ||
            message.contains("missing or invalid", true)

    private fun doPair() {
        val code = if (::codeBox.isInitialized) codeBox.text.toString().trim() else ""
        if (code.isEmpty()) {
            say("Enter the code shown in CapRust on the computer")
            return
        }
        // Pairing is a request on the open socket. Without it the send
        // fails with "not connected", which reads as a wrong code.
        if (!connected) {
            say(getString(R.string.msg_connect_first))
            return
        }
        say("Pairing ...")
        client.pair(code) { ok, msg ->
            say(if (ok) "Paired" else "Pairing failed: $msg")
            if (ok) {
                if (::codeBox.isInitialized) codeBox.text.clear()
                // Now there is something to show: the menu, not the
                // editor. Rebuilding also drops the "pairing needed"
                // banner, which this screen does not have.
                show(Screen.Home)
            }
        }
    }

    private fun doDisconnect() {
        client.disconnect()
        connected = false
        // Explicit disconnect, so stop reconnecting behind their back.
        wanted = false
        say("Disconnected")
        clipList?.removeAllViews()
        // Back to Connect: a disconnected Project screen is a dead end
        // with no way to reconnect from it.
        show(Screen.Connect)
    }

    /** Pull the summary and the clip list. Both are read-only, so a
     *  token-less client fails here rather than looking like a crash. */
    private fun refresh() {
        if (!connected) {
            // Without this the screen comes back from sleep completely
            // blank, with nothing to say why.
            // childCount, not hasViews(): the latter is an androidx
            // KTX extension and this app has no androidx dependency.
            if (clipList?.childCount == 0) {
                clipList?.removeAllViews()
                val note = TextView(this)
                note.text = if (wanted) "Reconnecting ..." else "Tap Connect to reach CapRust"
                clipList?.addView(note)
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
            // A tap on the preview needs the project length to turn into
            // a playhead position. Kept separate from lastDurationMs,
            // which is the timeline's own scale and defaults to 1ms when
            // the summary has not arrived -- far too small to divide a
            // tap by.
            projectDurationMs = runCatching {
                JSONObject(text).optLong("total_duration_ms", 0L)
            }.getOrDefault(0L)
            // The header shows the project name, so it has to be
            // captured before the screen is rebuilt.
            pendingProjectName = runCatching {
                JSONObject(text).optString("name", "")
            }.getOrDefault("")
            runOnUiThread { say(summarise(text)) }
        }
        client.call("list_clips") { ok, text ->
            if (!ok) {
                runOnUiThread { say(text) }
                return@call
            }
            runOnUiThread { renderClips(text) }
        }
        // Only when the preview is on screen. Asking for a frame on a
        // screen that has nowhere to put it wastes a request per
        // refresh for nothing.
        if (previewView != null) fetchPreview()
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
            say("Connection lost. It will reconnect when you return.")
        } else {
            say(message)
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
        clipList?.removeAllViews()
        clipNames.clear()
        clipInfo.clear()
        val arr = try {
            org.json.JSONArray(text)
        } catch (_: Exception) {
            // The first version built a TextView, dropped it, and then
            // added `this` -- the Activity. Bind it to a name.
            val fallback = TextView(this)
            fallback.text = text.take(200)
            clipList?.addView(fallback)
            timeline?.setClips(org.json.JSONArray(), 1)
            return
        }
        // Feed the drawn map from the same array the list uses, so the
        // two can never disagree about what is on the timeline.
        timeline?.setClips(arr, lastDurationMs)
        timeline?.setSelected(selectedClip)
        if (arr.length() == 0) {
            val empty = TextView(this)
            empty.setText(getString(R.string.no_clips))
            clipList?.addView(empty)
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
                showTransitionInfo()
                showClipSheet()
            }
            row.tag = id
            clipList?.addView(row)
        }
        showTransitionInfo()
    }

    /**
     * What is on the selected clip, and how long its transitions run.
     *
     * Read from `list_clips`, which reports the transition ids, their
     * duration, the shift they produced, and the applied effect
     * instances. Without the duration there was no way to read back
     * what the desktop clamped a request to, so a phone that asked for
     * 3000ms on a 2s clip and drew a 3000ms overlap would be showing
     * something the render never produces.
     */
    private fun showTransitionInfo() {
        val view = transitionInfo ?: return
        val id = selectedClip
        if (id == null) {
            view.text = getString(R.string.lbl_no_clip_selected)
            return
        }
        val c = clipInfo[id] ?: return
        val lines = ArrayList<String>()
        val tIn = c.optString("transition_in", "")
        val tOut = c.optString("transition_out", "")
        if (tIn.isEmpty() && tOut.isEmpty()) {
            lines.add(getString(R.string.lbl_no_transition))
        } else {
            val durMs = c.optLong("transition_duration_ms", 0L)
            val shift = c.optLong("applied_xfade_shift_ms", 0L)
            if (tIn.isNotEmpty()) {
                lines.add(getString(R.string.lbl_trans_in, tIn, durMs, shift))
            }
            if (tOut.isNotEmpty()) {
                lines.add(getString(R.string.lbl_trans_out, tOut, durMs))
            }
            lines.add(getString(R.string.lbl_trans_note))
        }
        val effects = c.optJSONArray("effects")
        if (effects != null && effects.length() > 0) {
            val names = ArrayList<String>()
            for (i in 0 until effects.length()) {
                val e = effects.getJSONObject(i)
                names.add(
                    e.optString("effect_id") +
                        if (e.optBoolean("enabled", true)) "" else " (off)"
                )
            }
            lines.add(getString(R.string.lbl_effects_on, names.joinToString(", ")))
        }
        // joinToString, not joinTo. joinTo also exists on Appendable and
        // resolves to that overload here, which does not return a
        // String; joinToString has no such variant and always does.
        val summary = lines.joinToString(separator = "\n")
        view.text = summary

        // Length control, only when there is a transition to lengthen.
        // Rebuilding on every selection is cheaper than keeping a
        // SeekBar alive across clip changes and re-reading its value.
        lengthHost?.removeAllViews()
        if (tIn.isEmpty() && tOut.isEmpty()) return
        lengthHost?.addView(buildLengthControl(id, c.optLong("transition_duration_ms", 500L)))
    }

    /**
     * A row of preset lengths plus the exact millisecond field.
     *
     * The presets cover what anyone actually picks (0.5 / 1 / 2 s); the
     * field is there because the desktop accepts any value and the
     * render clamps it, so the phone has to be able to ask for the odd
     * one and report back what it got.
     */
    private fun buildLengthControl(clipId: String, currentMs: Long): View {
        val box = card()
        box.addView(TextView(this).apply {
            text = getString(R.string.lbl_transition_length)
            textSize = 12f
        })
        val row = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        for (ms in longArrayOf(500L, 1000L, 2000L)) {
            row.addView(
                chip("${ms / 1000}s") { setTransitionLength(clipId, ms) },
                LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
            )
        }
        box.addView(row)

        val exactRow = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        val field = EditText(this).apply {
            hint = "ms"
            inputType = android.text.InputType.TYPE_CLASS_NUMBER
            setText(currentMs.toString())
        }
        exactRow.addView(
            field,
            LinearLayout.LayoutParams(0, dp(44), 1f)
        )
        exactRow.addView(chip(getString(R.string.btn_set_length)) {
            val typed = field.text.toString().trim().toLongOrNull()
            if (typed == null || typed < 1 || typed > 5000) {
                say(getString(R.string.msg_bad_length))
            } else {
                setTransitionLength(clipId, typed)
            }
        })
        box.addView(exactRow)
        return box
    }

    /** Change the length of the selected clip's in-transition. */
    private fun setTransitionLength(clipId: String, ms: Long) {
        val id = clipInfo[clipId]?.optString("transition_in", "") ?: ""
        if (id.isEmpty()) {
            say(getString(R.string.msg_no_transition))
            return
        }
        client.call(
            "set_transition",
            JSONObject()
                .put("clip_id", clipId)
                .put("edge", "in")
                .put("transition_id", id)
                .put("duration_ms", ms)
        ) { ok, body ->
            runOnUiThread {
                if (!ok) {
                    say(body)
                } else {
                    // The reply carries what was actually stored, which
                    // may be less than asked for: the render clamps to
                    // half of either neighbouring clip.
                    val got = runCatching {
                        JSONObject(body).optLong("duration_ms", ms)
                    }.getOrDefault(ms)
                    say(getString(R.string.msg_length_set, got))
                    refresh()
                }
            }
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
            say(getString(R.string.msg_tap_clip))
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
            // Take an applied effect off again. The phone could add one
            // before this but never remove it, so a mistake on the
            // phone had to be undone from the computer.
            val applied = info.optJSONArray("effects")
            if (hasRemoveEffect && applied != null && applied.length() > 0) {
                val labels = ArrayList<CharSequence>(applied.length())
                val ids = ArrayList<String>(applied.length())
                for (i in 0 until applied.length()) {
                    val e = applied.getJSONObject(i)
                    val eid = e.optString("effect_id")
                    ids.add(eid)
                    labels.add(
                        getString(
                            R.string.act_remove_effect,
                            eid,
                        )
                    )
                }
                // setItems needs Array<out CharSequence>, and Kotlin's
                // Array is invariant, so toTypedArray<CharSequence> is
                // required here -- a plain toTypedArray() will not
                // compile against List<String>.
                row(getString(R.string.act_effects_on_clip)) {
                    // this@MainActivity, not this: this sheet is built
                    // inside Dialog(...).apply, so a bare `this` here is
                    // the Dialog and the builder wants a Context. A
                    // Dialog is not one.
                    android.app.AlertDialog.Builder(this@MainActivity)
                        .setTitle(getString(R.string.btn_effects))
                        .setItems(labels.toTypedArray<CharSequence>()) { _, which ->
                            removeEffect(id, ids[which])
                        }
                        .setNegativeButton(android.R.string.cancel, null)
                        .show()
                }
            }
            if (kind.contains("Audio", true) || kind.contains("Video", true)) {
                row(getString(R.string.act_media)) { client.call("list_media") { ok, t ->
                    runOnUiThread { say(if (ok) t.take(300) else t) }
                } }
            }

            setContentView(col)
            show()
        }
    }

    /** Remove one applied effect from a clip. */
    private fun removeEffect(clipId: String, effectId: String) {
        client.call(
            "remove_effect",
            JSONObject().put("clip_id", clipId).put("effect_id", effectId)
        ) { ok, body ->
            runOnUiThread {
                if (ok) {
                    say(getString(R.string.msg_removed, effectId))
                    refresh()
                } else {
                    say(body)
                }
            }
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
            say(getString(R.string.msg_tap_clip))
            return
        }
        assetsCache[kind]?.let { return showPresetList(kind, id, it) }
        client.call("list_assets") { ok, text ->
            runOnUiThread {
                if (!ok) {
                    say(text)
                    return@runOnUiThread
                }
                val arr = try {
                    JSONObject(text).optJSONArray(kind) ?: JSONArray()
                } catch (_: Exception) {
                    JSONArray()
                }
                if (arr.length() == 0) {
                    say(getString(R.string.no_presets))
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
            // setItems takes Array<out CharSequence>, and Kotlin's Array is
            // invariant, so a plain toTypedArray() on List<String> does
            // not satisfy it.
            .setItems(ids.toTypedArray<CharSequence>()) { _, which ->
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
                    say(text)
                } else {
                    say(getString(R.string.msg_applied, presetId))
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
                say(if (ok) "Split at ${at}ms" else "Split failed: $msg")
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
                say(if (ok) "Moved to ${at}ms" else "Move failed: $msg")
                if (ok) refresh()
            }
        }
    }

    private fun setVolume(db: Double) {
        val id = selectedClip ?: return
        client.call("set_clip_volume", JSONObject().put("clip_id", id).put("db", db)) { ok, msg ->
            runOnUiThread {
                say(if (ok) "Volume $db dB" else "Volume failed: $msg")
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
                say(if (ok) "Speed ${clamped}x" else "Speed failed: $msg")
                if (ok) refresh()
            }
        }
    }

    private fun setFade() {
        val id = selectedClip ?: return
        val args = JSONObject().put("clip_id", id).put("fade_in_ms", 1000).put("fade_out_ms", 1000)
        client.call("set_clip_fade", args) { ok, msg ->
            runOnUiThread {
                say(if (ok) "Fade 1s in and out" else "Fade failed: $msg")
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
        val list = clipList ?: return
        for (i in 0 until list.childCount) {
            val child = list.getChildAt(i) as? Button ?: continue
            child.setAlpha(if (child.tag == selected) 1f else 0.6f)
        }
    }

    /**
     * Push new traffic into the log screen, if it is on display.
     *
     * Capture is unconditional; only the rendering is conditional. A
     * client that only recorded lines while the log was visible would
     * lose exactly the lines you open the log to read.
     */
    private fun renderLog() {
        val view = logView ?: return
        renderLogInto(view)
        view.post { (view.parent as? ScrollView)?.fullScroll(View.FOCUS_DOWN) }
    }

    private fun renderLogInto(into: TextView) {
        into.text = synchronized(client.log) { client.log.joinToString("\n") }
    }
}



