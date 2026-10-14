# CapRust MCP Server

JSON-RPC 2.0 control surface for a CapRust project. Loads a `.caprust`
file and exposes read-only inspection plus mutating tools that go through
the same `UndoStack` as the UI.

Speaks newline-delimited JSON over either stdio (AI agents) or a TCP
socket (a phone, or the editor driving its own open session). The
protocol does not change between them.

## Two ways to run it

**In the app (new in 0.9.17).** `Settings → Paths` has a *Remote
control* panel. Tick *Allow remote control* and the running editor starts
serving on port 4711, bound to loopback. This is the bridge: it is the
live session, so a remote edit lands in the project you have open, on
the same undo stack, and Ctrl+Z on the computer takes it back.

**Standalone binary.** `caprust-mcp` also runs on its own. That one is a
separate headless process with its own `ProjectState`; it does not see a
running window, and two of them editing the same file overwrite each
other on save. Use it for CI and agents that only need a project file.

## Build

    cargo build --release -p caprust-mcp

Binary: `target/release/caprust-mcp.exe`.

## Standalone, stdio

    caprust-mcp F:/Dev/projects/MyVlog.caprust

Line-delimited JSON-RPC 2.0 on stdin, responses on stdout, logs on
stderr. No HTTP, no tokio.

## Standalone, TCP

    caprust-mcp --tcp F:/Dev/projects/MyVlog.caprust
    caprust-mcp --bind 0.0.0.0 --port 4711 F:/Dev/projects/MyVlog.caprust

Prints the bound port on stdout, so `--tcp` with no `--port` works
when the caller asked for an OS-chosen one.

**The default bind is loopback.** The standalone binary has no
authentication of its own, so `--bind 0.0.0.0` puts an
unauthenticated project-editing endpoint on the network. Do it only on a
network you trust. The in-app bridge is the one that pairs — see below.

One client at a time: a second connection waits rather than
interleaving, because the server owns a single `ProjectState` and a
single undo stack.

## Claude Desktop

Add to `%APPDATA%/Claude/claude_desktop_config.json`:

    {
      "mcpServers": {
        "caprust": {
          "command": "F:/Dev/projects/CapRust/target/release/caprust-mcp.exe",
          "args": ["F:/Dev/projects/MyVlog.caprust"]
        }
      }
    }

Restart Claude Desktop. A copy-paste template lives in
`docs/claude_desktop_config.json`.

## Pairing (in-app bridge only)

Loopback needs nothing. Opening the port to the network is a separate
tick — *Allow devices on this network* — because that is the point where
someone else can edit your project.

Once it is ticked, the bridge requires a six-digit code. *New code*
generates one; it is valid **five minutes** and **one try only**. A
successful exchange returns a persistent token, stored in settings, and
sent on every later request. *Revoke access* throws it away.

The token rides in the JSON params rather than a header: the transport
is newline-delimited JSON and has no header concept, and keeping the two
transports interchangeable is worth more than a tidier envelope.

## Tools

26, verified against `tools/list` on the built binary:

Read-only — `get_project_summary`, `list_tracks`, `list_clips`,
`list_media`, `list_assets`, `list_recent_projects`, `preview_status`,
`export_status`, `export_settings`, `preview_frame`, `preview_transport`.

Mutating, each call one undoable step — `move_clip`, `split_clip`,
`set_transition`, `set_clip_volume`, `set_clip_speed`, `set_clip_fade`,
`add_clip_to_timeline`, `apply_preset`, `add_media`, `undo`, `redo`,
`save_project`, `open_project`, `set_export_setting`, `start_export`.

`list_assets` returns both the ids and the accepted values for each
kind, so a client never has to hardcode the vocabulary.

### Things that touch `self`

`open_project`, `start_export` and the transport calls change the thing
the other tools read. `Ctx` holds `&mut` to live state, so they cannot
be applied mid-batch. The bridge records them during a batch and applies
them after the batch completes, in the order they arrived.

## Trust model

The server runs with your user's permissions and trusts its client.
`add_media` registers any file your account can read, and
`save_project` writes to the project path. Only connect clients you
trust, and do not point a client at a project file you did not create.

On the in-app bridge, the loopback/LAN split plus the pairing code are
the whole of the authentication. There is no transport encryption —
this is a local-network tool, not something to expose to a router's
WAN.

## Known limits

- Single client, single session. A second connection waits.
- **Loading a project from a remote client immediately starts playback
  on the computer.** Not yet diagnosed. The phone only sends transport
  on an explicit press, and `open_project` does not set it, so something
  else is doing it.
- **`list_recent_projects` returns an empty array on the desktop** even
  though the list is persisted. Root cause unknown; the response
  carries a `count` so you can tell "the desktop has no entries" from
  "the call failed".
- Standalone only: `add_media` does not run ffprobe. New items land with
  `duration_ms = 0` until the project is opened in the editor.
- Standalone only: the undo stack starts empty per server session.
- The Android companion app is built in CI and shipped through nightly
  builds. It is not part of a stable release and has not had a full pass
  on a device.