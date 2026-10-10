# Community templates

Starter timeline templates shipped with CapRust. They are embedded in
the binary at build time (`crates/core/src/template.rs`), so adding a
file here + registering it there is all it takes.

## Contributing a template

1. In the app: arrange text clips (intro card, lower third, outro —
   media-free templates apply cleanly everywhere), then Templates tab
   → **Save as template**.
2. Copy the `.caprust-template` file from
   `%APPDATA%/CapRust/templates/` into this folder.
3. Scrub anything personal (project name is fine, it becomes the
   template name).
4. Register it in `bundled_templates()` in
   `crates/core/src/template.rs` (id, display name, include path).
5. Open a PR. Text-only templates are preferred for starters (no
   missing-media friction); templates referencing media work too —
   users relink on apply through the normal dialog.
