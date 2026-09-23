//! Fluent localization with runtime language switching.

use fluent_bundle::{FluentBundle, FluentResource};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use unic_langid::LanguageIdentifier;

thread_local! {
    static BUNDLES: RefCell<HashMap<String, FluentBundle<FluentResource>>> = RefCell::new({
        let mut map = HashMap::new();
        for (lang, ftl) in [
            ("en", include_str!("locales/en.ftl")),
            ("hr", include_str!("locales/hr.ftl")),
        ] {
            let resource = FluentResource::try_new(ftl.to_string()).expect("valid FTL");
            let lang_id: LanguageIdentifier = lang.parse().unwrap();
            let mut bundle = FluentBundle::new(vec![lang_id]);
            bundle.add_resource(resource).expect("add FTL resource");
            map.insert(lang.to_string(), bundle);
        }
        map
    });

    /// Current UI language, set once per frame by the app.
    static CURRENT: RefCell<String> = RefCell::new(String::from("en"));

    /// When true, `t` formats with parameters via `t_args`.
    static _UNUSED: Cell<()> = const { Cell::new(()) };
}

/// Set the active language for this frame. Call from the app's `update()`.
pub fn set_current_lang(lang: &str) {
    CURRENT.with(|c| {
        let mut c = c.borrow_mut();
        if c.as_str() != lang {
            *c = lang.to_string();
        }
    });
}

/// Translate using the current language.
pub fn t(key: &str) -> String {
    CURRENT.with(|c| t_lang(key, &c.borrow()))
}

/// Explicit-language variant (kept for compatibility).
pub fn t_lang(key: &str, lang: &str) -> String {
    BUNDLES.with(|bundles| {
        let bundles = bundles.borrow();
        if let Some(bundle) = bundles.get(lang) {
            if let Some(msg) = bundle.get_message(key) {
                if let Some(pattern) = msg.value() {
                    let mut errors = vec![];
                    return bundle
                        .format_pattern(pattern, None, &mut errors)
                        .to_string();
                }
            }
        }
        key.to_string()
    })
}

pub fn detect_locale() -> String {
    sys_locale::get_locale()
        .map(|l| l.split('-').next().unwrap_or("en").to_string())
        .filter(|l| matches!(l.as_str(), "en" | "hr"))
        .unwrap_or_else(|| "en".into())
}

pub const LANGUAGES: &[(&str, &str)] = &[("en", "English"), ("hr", "Hrvatski")];
