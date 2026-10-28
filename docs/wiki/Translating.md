# Translating

CapRust supports 5 languages: English, Croatian, Spanish, Italian, German.

## How to translate

1. Copy `crates/i18n/src/locales/en.ftl` to your language code (e.g., `fr.ftl`).
2. Translate the values (right side of `=`).
3. Register the language in `crates/i18n/src/lib.rs`.
4. Run `cargo test -p caprust-i18n` to verify key parity.
5. Open a PR.

## Notes

- Untranslated keys render as raw keys, so even partial translations help.
- The i18n guard test checks that all locale files have the same keys.
- Keep `#` comments for context.
EOF