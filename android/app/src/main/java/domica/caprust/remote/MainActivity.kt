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
 * Every network call runs on one background thread. Responses come back
 * to the main thread through [post], because touching a View from the
 * socket thread throws.
 */
class Client(private val ctx: Context) {

    private val io = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())

    private var socket: Socket? = null
    private var writer: BufferedWriter? = null
    private var reader: BufferedReader? = null
    private var nextId = 1

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
    fun connect(host: String, port: Int, then: (Boolean, String) -> Unit) = io.execute {
        try {
            closeQuietly()
            val s = Socket()
            // Short connect timeout: a wrong IP should say so in a few
            // seconds rather than hanging the button forever.
            s.connect(InetSocketAddress(host, port), 4000)
            s.soTimeout = 8000
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
                done(false, JSONObject(), why)
            }
        }

    fun disconnect() = io.execute {
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
        client = Client(this).also { it.loadToken() }
        client.onEvent = { renderLog() }
        setContentView(build())
        refresh()
    }

    /**
     * Re-establish the socket when coming back to the foreground.
     *
     * Screen off tears the connection down; without this the app comes
     * back looking connected while every later call fails on a dead
     * socket.
     */
    override fun onResume() {
        super.onResume()
        if (connected || !wanted) return
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
        actions2.addView(btn(getString(R.string.btn_split)) { splitSelected() })
        actions2.addView(btn(getString(R.string.btn_disconnect)) { doDisconnect() })
        root.addView(actions2)

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
        val logToggle = btn(getString(R.string.btn_toggle_log)) {
            logView.visibility =
                if (logView.visibility == View.VISIBLE) View.GONE else View.VISIBLE
        }
        root.addView(logToggle)
        val logScroll = ScrollView(this)
        logView = TextView(this).apply {
            textSize = 10f
            setTypeface(android.graphics.Typeface.MONOSPACE)
            setTextIsSelectable(true)
        }
        logScroll.addView(logView)
        root.addView(logScroll, LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, dp(140)))

        return root
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
        val arr = try {
            org.json.JSONArray(text)
        } catch (_: Exception) {
            // The first version built a TextView, dropped it, and then
            // added `this` -- the Activity. Bind it to a name.
            val fallback = TextView(this)
            fallback.text = text.take(200)
            clipList.addView(fallback)
            return
        }
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
            val row = btn(label) {
                selectedClip = id
                dimOthers(id)
            }
            row.tag = id
            clipList.addView(row)
        }
    }

    /** Split the selected clip at the midpoint of its duration.
     *
     *  A timestamp picker is not worth the surface area for v1: the
     *  phone is a remote control for a timeline that is far easier to
     *  aim at on the desktop. The midpoint is a safe, reversible
     *  operation and undo takes it back.
     */
    private fun splitSelected() {
        val id = selectedClip
        if (id == null) {
            status.text = "Tap a clip first"
            return
        }
        client.call("list_clips") { ok, text ->
            if (!ok) return@call
            try {
                val arr = org.json.JSONArray(text)
                for (i in 0 until arr.length()) {
                    val c = arr.getJSONObject(i)
                    if (c.optString("id") != id) continue
                    val start = c.optLong("start_time_ms")
                    val dur = c.optLong("duration_ms")
                    val at = start + dur / 2
                    client.call(
                        // The argument is `at_ms`, matching
                        // SplitClipCommand. `time_ms` was silently
                        // ignored as a missing parameter.
                        "split_clip",
                        JSONObject().put("clip_id", id).put("at_ms", at)
                    ) { ok2, msg ->
                        status.text = if (ok2) "Split at ${at}ms" else "Split failed: $msg"
                        if (ok2) refresh()
                    }
                    return@call
                }
                status.text = "Clip is no longer on the timeline"
            } catch (_: Exception) {
                status.text = "Could not read the clip list"
            }
        }
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