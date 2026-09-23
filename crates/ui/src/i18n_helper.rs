//! Short alias for i18n lookups inside the UI crate.
//! Use `tr("key")` in place of `caprust_i18n::t("key")`.

pub fn tr(key: &str) -> String {
    caprust_i18n::t(key)
}
