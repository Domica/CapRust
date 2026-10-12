//! Short alias for i18n lookups inside the UI crate.
//! Use `tr("key")` in place of `caprust_i18n::t("key")`.

pub fn tr(key: &str) -> String {
    caprust_i18n::t(key)
}

/// `tr` with Fluent variables filled in, for strings that interpolate
/// a value. See `caprust_i18n::t_with`.
pub fn tr_with(key: &str, args: &[(&str, &str)]) -> String {
    caprust_i18n::t_with(key, args)
}
