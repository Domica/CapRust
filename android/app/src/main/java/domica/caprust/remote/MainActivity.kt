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

    /**
     * Adopt a token stored for a different computer.
     *
     * Needed because the token belongs to a desktop, not to the app: a
     * pairing earned on one machine is rejected by another, so loading
     * the saved one on switch is the whole point of saving it.
     */
    fun setToken(v: String?) {
        token = v
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
    private enum class Screen { Connect, Project, Projects, Log, Settings, Connections }

    private var current = Screen.Connect

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
        client.onEvent = { renderLog() }
        // Land on Project when there is a stored token, so returning
        // from the launcher does not send a paired user back to a
        // Connect form they have already filled in. `wanted` alone is
        // not enough: it records that a connect was attempted, which is
        // also true of a session that connected and never paired.
        current = if (wanted && sharedPrefsHost().isNotEmpty() && client.isPaired()) {
            Screen.Project
        } else {
            Screen.Connect
        }
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
                Screen.Project -> buildProject()
                Screen.Projects -> buildProjects()
                Screen.Log -> buildLog()
                Screen.Settings -> buildSettings()
                Screen.Connections -> buildConnections()
            }
        )
        if (screen == Screen.Project) refresh()
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
    private fun back(): Screen = Screen.Connect

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
        if (current != Screen.Connect) {
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
                    // Same rule as a manual connect: an open socket
                    // without a token has nothing to show, and the
                    // pairing code is only enterable on Connect.
                    if (current == Screen.Connect && client.isPaired()) show(Screen.Project)
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
            btn(getString(R.string.btn_open)) { show(Screen.Projects) },
            btn(getString(R.string.btn_log)) { show(Screen.Log) },
            btn(getString(R.string.btn_settings)) { show(Screen.Settings) },
        ))
        root.addView(status)

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
        val wide = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        wide.addView(root, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 1f))
        val side = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(8), 0, dp(0), 0)
        }
        side.addView(
            selectedInfo(),
            LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            )
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
                    list.addView(TextView(this@MainActivity).apply {
                        text = getString(R.string.no_projects)
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
                // Only leave for the editor when there is something to
                // show there. An open socket is not the same as a
                // usable session: without a token every tool call comes
                // back "missing token", and jumping there strands the
                // user on a screen that cannot accept the pairing code
                // that would fix it.
                if (client.isPaired()) show(Screen.Project)
                else say(getString(R.string.msg_need_pair))
            }
        }
    }

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
                // Now there is something to show. Rebuilding the screen
                // also drops the "pairing needed" banner, since this
                // one would still be built without it.
                show(Screen.Project)
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
                showClipSheet()
            }
            row.tag = id
            clipList?.addView(row)
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
            if (kind.contains("Audio", true) || kind.contains("Video", true)) {
                row(getString(R.string.act_media)) { client.call("list_media") { ok, t ->
                    runOnUiThread { say(if (ok) t.take(300) else t) }
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



