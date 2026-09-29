# CapRust MCP Server

Standalone JSON-RPC 2.0 server over stdio. Loads a `.caprust`
project file and exposes read-only inspection plus mutating tools
that go through the same `UndoStack` as the UI.

## Build

    cargo build --release -p caprust-mcp

Binary: `target/release/caprust-mcp.exe` (Windows).

## Run

    caprust-mcp F:/Dev/projects/MyVlog.caprust

Line-delimited JSON-RPC 2.0 on stdin, responses on stdout, logs on
stderr. No HTTP, no socket, no tokio.

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
