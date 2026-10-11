//! Built-in sound-effects library (Phase #38).
//!
//! A curated catalog of Mixkit SFX (Free License: commercial OK, no
//! attribution, no signup). Files download on demand into
//! `%APPDATA%/CapRust/sfx/` and auto-import into the media bin as
//! audio items — from there drag-to-timeline and preview work through
//! the existing media paths, so this module owns only the catalog,
//! the cache dir and the download job.

/// One downloadable sound effect.
#[derive(Debug, Clone)]
pub struct SfxEntry {
    /// Stable id, also the cache filename stem.
    pub id: &'static str,
    /// Display name in the SFX tab.
    pub name: &'static str,
    /// Category for the left-column filter (whoosh, impact, ...).
    pub category: &'static str,
    /// Direct MP3 URL (Mixkit CDN, no auth).
    pub url: &'static str,
}

/// Curated catalog. Filled in from the verified URL list; every entry
/// below was HEAD-checked at curation time.
pub const CATALOG: &[SfxEntry] = &[
    SfxEntry {
        id: "sfx-01",
        name: "Air woosh",
        category: "whoosh",
        url: "https://assets.mixkit.co/active_storage/sfx/1489/1489-preview.mp3",
    },
    SfxEntry {
        id: "sfx-02",
        name: "Cinematic whoosh fast transition",
        category: "whoosh",
        url: "https://assets.mixkit.co/active_storage/sfx/1492/1492-preview.mp3",
    },
    SfxEntry {
        id: "sfx-03",
        name: "Cinematic whoosh stutter",
        category: "whoosh",
        url: "https://assets.mixkit.co/active_storage/sfx/787/787-preview.mp3",
    },
    SfxEntry {
        id: "sfx-04",
        name: "Arrow whoosh",
        category: "whoosh",
        url: "https://assets.mixkit.co/active_storage/sfx/1491/1491-preview.mp3",
    },
    SfxEntry {
        id: "sfx-05",
        name: "Cinematic wind swoosh",
        category: "swoosh",
        url: "https://assets.mixkit.co/active_storage/sfx/1471/1471-preview.mp3",
    },
    SfxEntry {
        id: "sfx-06",
        name: "Sword slash swoosh",
        category: "swoosh",
        url: "https://assets.mixkit.co/active_storage/sfx/1476/1476-preview.mp3",
    },
    SfxEntry {
        id: "sfx-07",
        name: "Wood hard hit",
        category: "impact",
        url: "https://assets.mixkit.co/active_storage/sfx/2182/2182-preview.mp3",
    },
    SfxEntry {
        id: "sfx-08",
        name: "Movie trailer epic impact",
        category: "impact",
        url: "https://assets.mixkit.co/active_storage/sfx/2908/2908-preview.mp3",
    },
    SfxEntry {
        id: "sfx-09",
        name: "Horror impact",
        category: "impact",
        url: "https://assets.mixkit.co/active_storage/sfx/773/773-preview.mp3",
    },
    SfxEntry {
        id: "sfx-10",
        name: "Cinematic trailer riser",
        category: "riser",
        url: "https://assets.mixkit.co/active_storage/sfx/790/790-preview.mp3",
    },
    SfxEntry {
        id: "sfx-11",
        name: "Short space stutter intro riser",
        category: "riser",
        url: "https://assets.mixkit.co/active_storage/sfx/1144/1144-preview.mp3",
    },
    SfxEntry {
        id: "sfx-12",
        name: "Fast double click on mouse",
        category: "click",
        url: "https://assets.mixkit.co/active_storage/sfx/275/275-preview.mp3",
    },
    SfxEntry {
        id: "sfx-13",
        name: "Select click",
        category: "click",
        url: "https://assets.mixkit.co/active_storage/sfx/1109/1109-preview.mp3",
    },
    SfxEntry {
        id: "sfx-14",
        name: "Hard pop click",
        category: "pop",
        url: "https://assets.mixkit.co/active_storage/sfx/2364/2364-preview.mp3",
    },
    SfxEntry {
        id: "sfx-15",
        name: "Electric pop",
        category: "pop",
        url: "https://assets.mixkit.co/active_storage/sfx/2365/2365-preview.mp3",
    },
    SfxEntry {
        id: "sfx-16",
        name: "Game success alert",
        category: "success",
        url: "https://assets.mixkit.co/active_storage/sfx/2039/2039-preview.mp3",
    },
    SfxEntry {
        id: "sfx-17",
        name: "Success software tone",
        category: "success",
        url: "https://assets.mixkit.co/active_storage/sfx/2865/2865-preview.mp3",
    },
    SfxEntry {
        id: "sfx-18",
        name: "Correct answer tone",
        category: "notification",
        url: "https://assets.mixkit.co/active_storage/sfx/2870/2870-preview.mp3",
    },
    SfxEntry {
        id: "sfx-19",
        name: "Bell notification",
        category: "notification",
        url: "https://assets.mixkit.co/active_storage/sfx/933/933-preview.mp3",
    },
    SfxEntry {
        id: "sfx-20",
        name: "Confirmation tone",
        category: "notification",
        url: "https://assets.mixkit.co/active_storage/sfx/2867/2867-preview.mp3",
    },
    SfxEntry {
        id: "sfx-21",
        name: "Positive notification",
        category: "notification",
        url: "https://assets.mixkit.co/active_storage/sfx/951/951-preview.mp3",
    },
    SfxEntry {
        id: "sfx-22",
        name: "Cinematic whoosh deep impact",
        category: "cinematic-boom",
        url: "https://assets.mixkit.co/active_storage/sfx/1143/1143-preview.mp3",
    },
    SfxEntry {
        id: "sfx-23",
        name: "Cinematic mystery trailer drum hit",
        category: "cinematic-boom",
        url: "https://assets.mixkit.co/active_storage/sfx/546/546-preview.mp3",
    },
    SfxEntry {
        id: "sfx-24",
        name: "Small electric glitch",
        category: "glitch",
        url: "https://assets.mixkit.co/active_storage/sfx/2595/2595-preview.mp3",
    },
    SfxEntry {
        id: "sfx-25",
        name: "Cinematic sci fi glitch",
        category: "glitch",
        url: "https://assets.mixkit.co/active_storage/sfx/1021/1021-preview.mp3",
    },
    SfxEntry {
        id: "sfx-26",
        name: "Short bass hit",
        category: "drum",
        url: "https://assets.mixkit.co/active_storage/sfx/2299/2299-preview.mp3",
    },
    SfxEntry {
        id: "sfx-27",
        name: "Drum bass hit",
        category: "drum",
        url: "https://assets.mixkit.co/active_storage/sfx/2294/2294-preview.mp3",
    },
    SfxEntry {
        id: "sfx-28",
        name: "Fast small sweep transition",
        category: "transition",
        url: "https://assets.mixkit.co/active_storage/sfx/166/166-preview.mp3",
    },
    SfxEntry {
        id: "sfx-29",
        name: "Short transition sweep",
        category: "transition",
        url: "https://assets.mixkit.co/active_storage/sfx/175/175-preview.mp3",
    },
    SfxEntry {
        id: "sfx-30",
        name: "Quick metal transition sweep",
        category: "transition",
        url: "https://assets.mixkit.co/active_storage/sfx/2639/2639-preview.mp3",
    },
];

/// Distinct categories in catalog order, for the filter column.
pub fn categories() -> Vec<&'static str> {
    let mut out = Vec::new();
    for e in CATALOG {
        if !out.contains(&e.category) {
            out.push(e.category);
        }
    }
    out
}

/// SFX cache dir: `%APPDATA%/CapRust/sfx/`.
pub fn cache_dir() -> std::path::PathBuf {
    let base = std::env::var("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("CapRust").join("sfx")
}

/// Local path of a catalog entry (`<id>.mp3` in the cache dir).
pub fn local_path(entry: &SfxEntry) -> std::path::PathBuf {
    cache_dir().join(format!("{}.mp3", entry.id))
}

/// True when the entry is fully downloaded (non-empty file; a
/// `.part` leftover does not count).
pub fn is_downloaded(entry: &SfxEntry) -> bool {
    let p = local_path(entry);
    p.is_file() && p.metadata().map(|m| m.len() > 0).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_ids_are_unique_and_urls_look_right() {
        let mut ids = std::collections::HashSet::new();
        for e in CATALOG {
            assert!(ids.insert(e.id), "duplicate sfx id {}", e.id);
            assert!(!e.name.is_empty());
            assert!(!e.category.is_empty());
            assert!(
                e.url
                    .starts_with("https://assets.mixkit.co/active_storage/sfx/"),
                "unexpected SFX url {}",
                e.url
            );
            assert!(e.url.ends_with("-preview.mp3"));
        }
        assert!(
            !categories().is_empty(),
            "catalog must expose at least one category"
        );
    }

    #[test]
    fn cache_paths_stay_inside_cache_dir() {
        for e in CATALOG {
            let p = local_path(e);
            assert!(p.starts_with(cache_dir()));
            assert_eq!(p.extension().and_then(|x| x.to_str()), Some("mp3"));
        }
    }
}
