# Translating CapRust

CapRust ships English, Croatian, Spanish, Italian and German via
[Fluent](https://projectfluent.org). Every user-visible string
resolves through `tr("key")` — there are no hardcoded UI strings by
design.

Want to help with a new language (or fix an existing one)? Reply in
the [translations discussion](https://github.com/Domica/CapRust/discussions/46)
so two people don't duplicate work.

## Files

```
crates/i18n/src/locales/en.ftl   # source of truth, always complete
crates/i18n/src/locales/hr.ftl   # Croatian translation
crates/i18n/src/locales/es.ftl   # Spanish translation
crates/i18n/src/locales/it.ftl   # Italian translation
crates/i18n/src/locales/de.ftl   # German translation
crates/i18n/src/lib.rs           # registration + language list
```

Both `.ftl` files are UTF-8. Non-ASCII may be written raw (`č`) or as
`\u` escapes (`\u010d`) — the file mixes both historically; match the
lines around yours.

## Adding or changing a string

1. Add the key to **both** files in the same commit. CI does not check
   this yet, so do it by discipline: no key lands in one language only.
2. Key naming: `area-thing-detail`, e.g. `menu-file-open`,
   `props-sound-denoise`, `toast-export-failed`, `asset-effect-blur`.
3. Parameters use Fluent placeholders: `toast-caption-failed =
   Transcription failed: { $err }`, formatted at the call site.
4. In code: `tr("my-key")` (needs `use crate::i18n_helper::tr;`).
   For tooltips on icon buttons pass the already-translated string.

## Testing a translation

Switch language at runtime in Settings → Appearance → Language, no
restart needed. Walk the panel you touched; untranslated keys render
as the raw key, which is how you spot misses.

## Adding a whole new language

1. Copy `en.ftl` → `xx.ftl` (language code) and translate values.
   Keep keys, placeholders (`{ $var }`) and comments intact.
2. Register in `crates/i18n/src/lib.rs`: add `("xx", include_str!(...))`
   to `BUNDLES`, extend `LANGUAGES`, and extend the `matches!` filter
   in `detect_locale`, otherwise the OS locale never resolves to it.
3. Open a PR with the new file + registration. A second speaker of the
   language reviewing is appreciated but not required.
