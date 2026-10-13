# CapRust MCP Server

Standalone JSON-RPC 2.0 server. Loads a `.caprust` project file and
exposes read-only inspection plus mutating tools that go through the
same `UndoStack` as the UI.

Speaks newline-delimited JSON over either stdio (AI agents) or a TCP
socket (a phone driving the same tool surface). The protocol does not
change between them.

## Build

    cargo build --release -p caprust-mcp

Binary: `target/release/caprust-mcp.exe` (Windows).

## Run

    caprust-mcp F:/Dev/projects/MyVlog.caprust

Line-delimited JSON-RPC 2.0 on stdin, responses on stdout, logs on
stderr. No HTTP, no tokio.

## TCP mode

    caprust-mcp --tcp F:/Dev/projects/MyVlog.caprust
    caprust-mcp --bind 0.0.0.0 --port 4711 F:/Dev/projects/MyVlog.caprust

Prints the bound port on stdout, so `--tcp` with no `--port` works
when the caller asked for an OS-chosen one.

**The default bind is loopback.** `--bind 0.0.0.0` puts an
unauthenticated project-editing endpoint on the network; there is no
authentication yet, so do it only on a network you trust. One client
at a time: a second connection waits rather than interleaving, because
the server owns a single `ProjectState` and a single undo stack.

This is still a separate headless process. It does not share state
with a running CapRust window — two of them editing the same file will
overwrite each other on save.

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

## Trust model

The server runs with your user's permissions and trusts its client.
`add_media` registers any file your account can read, and
`save_project` writes to the project path. Only connect clients you
trust, and do not point a client at a project file you did not create.

## Tools

Read-only: `get_project_summary`, `list_tracks`, `list_clips`,
`list_media`.

Mutating (each call = one undoable step): `add_media`,
`add_clip_to_timeline`, `move_clip`, `split_clip`,
`set_transition`, `set_clip_volume`, `set_clip_speed`,
`set_clip_fade`, `undo`, `redo`, `save_project`.

## Known limits

- Single client, single session. No live sync with a running GUI.
- `add_media` does not run ffprobe. New items land with
  `duration_ms = 0` until the project is opened in the editor.
- Undo stack starts empty per server session.
- Not launched by the editor. Standalone binary.
