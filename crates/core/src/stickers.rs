//! Built-in sticker library. Generated -- do not hand-edit.
//!
//! Produced by `crates/app/assets/gen_stickers.py`. Every id here
//! has a PNG beside this file and is embedded in the binary, so a
//! fresh install has the whole library with no download step.

/// One sticker: a stable id, a display name, and the file bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sticker {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub png: &'static [u8],
}

pub const CATALOG: &[Sticker] = &[
    Sticker {
        id: "circle",
        name: "Circle",
        category: "shape",
        png: include_bytes!("../../app/assets/stickers/circle.png"),
    },
    Sticker {
        id: "ring",
        name: "Ring",
        category: "shape",
        png: include_bytes!("../../app/assets/stickers/ring.png"),
    },
    Sticker {
        id: "rounded-square",
        name: "Rounded Square",
        category: "shape",
        png: include_bytes!("../../app/assets/stickers/rounded-square.png"),
    },
    Sticker {
        id: "triangle",
        name: "Triangle",
        category: "shape",
        png: include_bytes!("../../app/assets/stickers/triangle.png"),
    },
    Sticker {
        id: "hexagon",
        name: "Hexagon",
        category: "shape",
        png: include_bytes!("../../app/assets/stickers/hexagon.png"),
    },
    Sticker {
        id: "pentagon",
        name: "Pentagon",
        category: "shape",
        png: include_bytes!("../../app/assets/stickers/pentagon.png"),
    },
    Sticker {
        id: "diamond",
        name: "Diamond",
        category: "shape",
        png: include_bytes!("../../app/assets/stickers/diamond.png"),
    },
    Sticker {
        id: "star",
        name: "Star",
        category: "shape",
        png: include_bytes!("../../app/assets/stickers/star.png"),
    },
    Sticker {
        id: "hex-star",
        name: "Six Point Star",
        category: "shape",
        png: include_bytes!("../../app/assets/stickers/hex-star.png"),
    },
    Sticker {
        id: "burst",
        name: "Burst",
        category: "shape",
        png: include_bytes!("../../app/assets/stickers/burst.png"),
    },
    Sticker {
        id: "blob",
        name: "Blob",
        category: "shape",
        png: include_bytes!("../../app/assets/stickers/blob.png"),
    },
    Sticker {
        id: "chevron",
        name: "Chevron",
        category: "shape",
        png: include_bytes!("../../app/assets/stickers/chevron.png"),
    },
    Sticker {
        id: "cross",
        name: "Cross",
        category: "shape",
        png: include_bytes!("../../app/assets/stickers/cross.png"),
    },
    Sticker {
        id: "capsule",
        name: "Capsule",
        category: "shape",
        png: include_bytes!("../../app/assets/stickers/capsule.png"),
    },
    Sticker {
        id: "teardrop",
        name: "Teardrop",
        category: "shape",
        png: include_bytes!("../../app/assets/stickers/teardrop.png"),
    },
    Sticker {
        id: "moon",
        name: "Moon",
        category: "shape",
        png: include_bytes!("../../app/assets/stickers/moon.png"),
    },
    Sticker {
        id: "sun",
        name: "Sun",
        category: "shape",
        png: include_bytes!("../../app/assets/stickers/sun.png"),
    },
    Sticker {
        id: "squircle",
        name: "Squircle",
        category: "shape",
        png: include_bytes!("../../app/assets/stickers/squircle.png"),
    },
    Sticker {
        id: "trapezoid",
        name: "Trapezoid",
        category: "shape",
        png: include_bytes!("../../app/assets/stickers/trapezoid.png"),
    },
    Sticker {
        id: "heart",
        name: "Heart",
        category: "heart",
        png: include_bytes!("../../app/assets/stickers/heart.png"),
    },
    Sticker {
        id: "heart-outline",
        name: "Heart Outline",
        category: "heart",
        png: include_bytes!("../../app/assets/stickers/heart-outline.png"),
    },
    Sticker {
        id: "heart-double",
        name: "Two Hearts",
        category: "heart",
        png: include_bytes!("../../app/assets/stickers/heart-double.png"),
    },
    Sticker {
        id: "heart-broken",
        name: "Broken Heart",
        category: "heart",
        png: include_bytes!("../../app/assets/stickers/heart-broken.png"),
    },
    Sticker {
        id: "heart-burst",
        name: "Heart Burst",
        category: "heart",
        png: include_bytes!("../../app/assets/stickers/heart-burst.png"),
    },
    Sticker {
        id: "heart-frame",
        name: "Heart Frame",
        category: "heart",
        png: include_bytes!("../../app/assets/stickers/heart-frame.png"),
    },
    Sticker {
        id: "heart-sparkle",
        name: "Heart Sparkle",
        category: "heart",
        png: include_bytes!("../../app/assets/stickers/heart-sparkle.png"),
    },
    Sticker {
        id: "heart-pulse",
        name: "Heartbeat",
        category: "heart",
        png: include_bytes!("../../app/assets/stickers/heart-pulse.png"),
    },
    Sticker {
        id: "heart-globe",
        name: "Heart Globe",
        category: "heart",
        png: include_bytes!("../../app/assets/stickers/heart-globe.png"),
    },
    Sticker {
        id: "heart-arrow",
        name: "Heart Arrow",
        category: "heart",
        png: include_bytes!("../../app/assets/stickers/heart-arrow.png"),
    },
    Sticker {
        id: "arrow-right",
        name: "Arrow Right",
        category: "arrow",
        png: include_bytes!("../../app/assets/stickers/arrow-right.png"),
    },
    Sticker {
        id: "arrow-left",
        name: "Arrow Left",
        category: "arrow",
        png: include_bytes!("../../app/assets/stickers/arrow-left.png"),
    },
    Sticker {
        id: "arrow-up",
        name: "Arrow Up",
        category: "arrow",
        png: include_bytes!("../../app/assets/stickers/arrow-up.png"),
    },
    Sticker {
        id: "arrow-down",
        name: "Arrow Down",
        category: "arrow",
        png: include_bytes!("../../app/assets/stickers/arrow-down.png"),
    },
    Sticker {
        id: "arrow-double",
        name: "Double Arrow",
        category: "arrow",
        png: include_bytes!("../../app/assets/stickers/arrow-double.png"),
    },
    Sticker {
        id: "arrow-curved",
        name: "Curved Arrow",
        category: "arrow",
        png: include_bytes!("../../app/assets/stickers/arrow-curved.png"),
    },
    Sticker {
        id: "arrow-block",
        name: "Block Arrow",
        category: "arrow",
        png: include_bytes!("../../app/assets/stickers/arrow-block.png"),
    },
    Sticker {
        id: "arrow-circle",
        name: "Circle Arrow",
        category: "arrow",
        png: include_bytes!("../../app/assets/stickers/arrow-circle.png"),
    },
    Sticker {
        id: "bolt",
        name: "Lightning Bolt",
        category: "arrow",
        png: include_bytes!("../../app/assets/stickers/bolt.png"),
    },
    Sticker {
        id: "arrow-tail",
        name: "Motion Arrow",
        category: "arrow",
        png: include_bytes!("../../app/assets/stickers/arrow-tail.png"),
    },
    Sticker {
        id: "arrow-zigzag",
        name: "Zigzag",
        category: "arrow",
        png: include_bytes!("../../app/assets/stickers/arrow-zigzag.png"),
    },
    Sticker {
        id: "pin-heart",
        name: "Heart Pin",
        category: "arrow",
        png: include_bytes!("../../app/assets/stickers/pin-heart.png"),
    },
    Sticker {
        id: "pin-round",
        name: "Round Pin",
        category: "arrow",
        png: include_bytes!("../../app/assets/stickers/pin-round.png"),
    },
    Sticker {
        id: "arrow-retro",
        name: "Retro Arrow",
        category: "arrow",
        png: include_bytes!("../../app/assets/stickers/arrow-retro.png"),
    },
    Sticker {
        id: "sparkle",
        name: "Sparkle",
        category: "sparkle",
        png: include_bytes!("../../app/assets/stickers/sparkle.png"),
    },
    Sticker {
        id: "sparkle-small",
        name: "Small Sparkle",
        category: "sparkle",
        png: include_bytes!("../../app/assets/stickers/sparkle-small.png"),
    },
    Sticker {
        id: "star5",
        name: "Five Point Star",
        category: "sparkle",
        png: include_bytes!("../../app/assets/stickers/star5.png"),
    },
    Sticker {
        id: "star8",
        name: "Eight Point Star",
        category: "sparkle",
        png: include_bytes!("../../app/assets/stickers/star8.png"),
    },
    Sticker {
        id: "star-outline",
        name: "Star Outline",
        category: "sparkle",
        png: include_bytes!("../../app/assets/stickers/star-outline.png"),
    },
    Sticker {
        id: "shooting-star",
        name: "Shooting Star",
        category: "sparkle",
        png: include_bytes!("../../app/assets/stickers/shooting-star.png"),
    },
    Sticker {
        id: "sparkle-trail",
        name: "Sparkle Trail",
        category: "sparkle",
        png: include_bytes!("../../app/assets/stickers/sparkle-trail.png"),
    },
    Sticker {
        id: "glow-dot",
        name: "Glow Dot",
        category: "sparkle",
        png: include_bytes!("../../app/assets/stickers/glow-dot.png"),
    },
    Sticker {
        id: "twinkle",
        name: "Twinkle",
        category: "sparkle",
        png: include_bytes!("../../app/assets/stickers/twinkle.png"),
    },
    Sticker {
        id: "star-stack",
        name: "Stacked Stars",
        category: "sparkle",
        png: include_bytes!("../../app/assets/stickers/star-stack.png"),
    },
    Sticker {
        id: "sparkle-ring",
        name: "Sparkle Ring",
        category: "sparkle",
        png: include_bytes!("../../app/assets/stickers/sparkle-ring.png"),
    },
    Sticker {
        id: "speech",
        name: "Speech Bubble",
        category: "speech",
        png: include_bytes!("../../app/assets/stickers/speech.png"),
    },
    Sticker {
        id: "speech-round",
        name: "Round Bubble",
        category: "speech",
        png: include_bytes!("../../app/assets/stickers/speech-round.png"),
    },
    Sticker {
        id: "thought",
        name: "Thought Bubble",
        category: "speech",
        png: include_bytes!("../../app/assets/stickers/thought.png"),
    },
    Sticker {
        id: "shout",
        name: "Shout Burst",
        category: "speech",
        png: include_bytes!("../../app/assets/stickers/shout.png"),
    },
    Sticker {
        id: "comic-burst",
        name: "Comic Burst",
        category: "speech",
        png: include_bytes!("../../app/assets/stickers/comic-burst.png"),
    },
    Sticker {
        id: "bubble-exclaim",
        name: "Exclamation Bubble",
        category: "speech",
        png: include_bytes!("../../app/assets/stickers/bubble-exclaim.png"),
    },
    Sticker {
        id: "bubble-question",
        name: "Question Bubble",
        category: "speech",
        png: include_bytes!("../../app/assets/stickers/bubble-question.png"),
    },
    Sticker {
        id: "bubble-heart",
        name: "Heart Bubble",
        category: "speech",
        png: include_bytes!("../../app/assets/stickers/bubble-heart.png"),
    },
    Sticker {
        id: "whisper",
        name: "Whisper",
        category: "speech",
        png: include_bytes!("../../app/assets/stickers/whisper.png"),
    },
    Sticker {
        id: "ribbon",
        name: "Ribbon",
        category: "speech",
        png: include_bytes!("../../app/assets/stickers/ribbon.png"),
    },
    Sticker {
        id: "tag",
        name: "Tag",
        category: "speech",
        png: include_bytes!("../../app/assets/stickers/tag.png"),
    },
    Sticker {
        id: "label",
        name: "Label",
        category: "speech",
        png: include_bytes!("../../app/assets/stickers/label.png"),
    },
    Sticker {
        id: "badge-new",
        name: "New Badge",
        category: "badge",
        png: include_bytes!("../../app/assets/stickers/badge-new.png"),
    },
    Sticker {
        id: "badge-hot",
        name: "Hot Badge",
        category: "badge",
        png: include_bytes!("../../app/assets/stickers/badge-hot.png"),
    },
    Sticker {
        id: "badge-sale",
        name: "Sale Badge",
        category: "badge",
        png: include_bytes!("../../app/assets/stickers/badge-sale.png"),
    },
    Sticker {
        id: "badge-check",
        name: "Check Badge",
        category: "badge",
        png: include_bytes!("../../app/assets/stickers/badge-check.png"),
    },
    Sticker {
        id: "badge-cross",
        name: "Cross Badge",
        category: "badge",
        png: include_bytes!("../../app/assets/stickers/badge-cross.png"),
    },
    Sticker {
        id: "badge-warning",
        name: "Warning",
        category: "badge",
        png: include_bytes!("../../app/assets/stickers/badge-warning.png"),
    },
    Sticker {
        id: "crown",
        name: "Crown",
        category: "badge",
        png: include_bytes!("../../app/assets/stickers/crown.png"),
    },
    Sticker {
        id: "percent",
        name: "Percent",
        category: "badge",
        png: include_bytes!("../../app/assets/stickers/percent.png"),
    },
    Sticker {
        id: "ribbon-circle",
        name: "Ribbon Badge",
        category: "badge",
        png: include_bytes!("../../app/assets/stickers/ribbon-circle.png"),
    },
    Sticker {
        id: "target",
        name: "Target",
        category: "badge",
        png: include_bytes!("../../app/assets/stickers/target.png"),
    },
    Sticker {
        id: "shield",
        name: "Shield Check",
        category: "badge",
        png: include_bytes!("../../app/assets/stickers/shield.png"),
    },
    Sticker {
        id: "frame-film",
        name: "Film Frame",
        category: "frame",
        png: include_bytes!("../../app/assets/stickers/frame-film.png"),
    },
    Sticker {
        id: "frame-polaroid",
        name: "Polaroid",
        category: "frame",
        png: include_bytes!("../../app/assets/stickers/frame-polaroid.png"),
    },
    Sticker {
        id: "frame-torn",
        name: "Torn Edge",
        category: "frame",
        png: include_bytes!("../../app/assets/stickers/frame-torn.png"),
    },
    Sticker {
        id: "frame-corners",
        name: "Corners",
        category: "frame",
        png: include_bytes!("../../app/assets/stickers/frame-corners.png"),
    },
    Sticker {
        id: "frame-round",
        name: "Round Border",
        category: "frame",
        png: include_bytes!("../../app/assets/stickers/frame-round.png"),
    },
    Sticker {
        id: "frame-rounded",
        name: "Rounded Border",
        category: "frame",
        png: include_bytes!("../../app/assets/stickers/frame-rounded.png"),
    },
    Sticker {
        id: "frame-gradient",
        name: "Gradient Border",
        category: "frame",
        png: include_bytes!("../../app/assets/stickers/frame-gradient.png"),
    },
    Sticker {
        id: "frame-comic",
        name: "Comic Border",
        category: "frame",
        png: include_bytes!("../../app/assets/stickers/frame-comic.png"),
    },
    Sticker {
        id: "frame-vintage",
        name: "Vintage Frame",
        category: "frame",
        png: include_bytes!("../../app/assets/stickers/frame-vintage.png"),
    },
    Sticker {
        id: "frame-tape",
        name: "Taped Frame",
        category: "frame",
        png: include_bytes!("../../app/assets/stickers/frame-tape.png"),
    },
    Sticker {
        id: "frame-scribble",
        name: "Scribble",
        category: "frame",
        png: include_bytes!("../../app/assets/stickers/frame-scribble.png"),
    },
    Sticker {
        id: "speed-lines",
        name: "Speed Lines",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/speed-lines.png"),
    },
    Sticker {
        id: "swoosh",
        name: "Swoosh",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/swoosh.png"),
    },
    Sticker {
        id: "blur-streak",
        name: "Blur Streak",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/blur-streak.png"),
    },
    Sticker {
        id: "impact",
        name: "Impact",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/impact.png"),
    },
    Sticker {
        id: "dust",
        name: "Dust",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/dust.png"),
    },
    Sticker {
        id: "lens",
        name: "Lens Flare",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/lens.png"),
    },
    Sticker {
        id: "zigzag-motion",
        name: "Zigzag Motion",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/zigzag-motion.png"),
    },
    Sticker {
        id: "sym-check",
        name: "Check",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/sym-check.png"),
    },
    Sticker {
        id: "sym-cross",
        name: "X",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/sym-cross.png"),
    },
    Sticker {
        id: "sym-plus",
        name: "Plus",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/sym-plus.png"),
    },
    Sticker {
        id: "sym-infinity",
        name: "Infinity",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/sym-infinity.png"),
    },
    Sticker {
        id: "sym-question",
        name: "Question Mark",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/sym-question.png"),
    },
    Sticker {
        id: "sym-exclaim",
        name: "Exclamation Mark",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/sym-exclaim.png"),
    },
    Sticker {
        id: "sym-hashtag",
        name: "Hashtag",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/sym-hashtag.png"),
    },
    Sticker {
        id: "sym-at",
        name: "At Sign",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/sym-at.png"),
    },
    Sticker {
        id: "droplet",
        name: "Droplet",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/droplet.png"),
    },
    Sticker {
        id: "leaf",
        name: "Leaf",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/leaf.png"),
    },
    Sticker {
        id: "cloud",
        name: "Cloud",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/cloud.png"),
    },
    Sticker {
        id: "flower",
        name: "Flower",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/flower.png"),
    },
    Sticker {
        id: "globe",
        name: "Globe",
        category: "mark",
        png: include_bytes!("../../app/assets/stickers/globe.png"),
    },
];

// ---- end of generated section ----

/// Directory the embedded PNGs are unpacked into.
///
/// The bytes live in the binary, but ffmpeg needs a path on disk, so a
/// sticker is materialised on first use. Same shape as the built-in LUT
/// cache: a per-install cache under %APPDATA%, written once, reused after.
pub fn cache_dir() -> std::path::PathBuf {
    std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
        .join("CapRust")
        .join("stickers")
}

/// The catalog entry for `id`, if there is one.
pub fn find(id: &str) -> Option<&'static Sticker> {
    CATALOG.iter().find(|s| s.id == id)
}

/// Write every embedded sticker that is not on disk yet. Idempotent.
pub fn materialize_all() -> std::io::Result<()> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)?;
    for s in CATALOG {
        let path = dir.join(format!("{}.png", s.id));
        if !path.exists() {
            std::fs::write(&path, s.png)?;
        }
    }
    Ok(())
}

/// Path to `id`'s PNG, writing it out if needed.
pub fn materialize(id: &str) -> std::io::Result<std::path::PathBuf> {
    let entry = find(id).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("no sticker called {id}"),
        )
    })?;
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.png", entry.id));
    if !path.exists() {
        std::fs::write(&path, entry.png)?;
    }
    Ok(path)
}

/// Pixel size of a sticker's PNG, read from its IHDR.
///
/// The host needs this to draw the marker's box and handle on screen and
/// to set `OverlayPlacement::aspect`. Every built-in is 256x256, but
/// hardcoding that would be wrong the moment one is not.
pub fn size(id: &str) -> Option<(u32, u32)> {
    let png = find(id)?.png;
    if png.len() < 24 || &png[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

/// Width / height, for `OverlayPlacement::aspect`.
pub fn aspect(id: &str) -> f32 {
    match size(id) {
        Some((w, h)) => w as f32 / h as f32,
        None => 1.0,
    }
}

/// Categories in the order the tab shows them.
pub fn categories() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for s in CATALOG {
        if !out.contains(&s.category) {
            out.push(s.category);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_id_is_unique_and_resolvable() {
        let mut ids: Vec<&str> = CATALOG.iter().map(|s| s.id).collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "duplicate sticker id");
        for s in CATALOG {
            assert_eq!(find(s.id), Some(s));
            assert!(!s.name.is_empty(), "{} has no name", s.id);
        }
    }

    #[test]
    fn every_sticker_carries_a_real_png() {
        // A PNG signature rather than just "non-empty": an empty file and
        // an HTML error page both pass a length check, and both fail later
        // inside ffmpeg with a message that does not name the sticker.
        for s in CATALOG {
            assert!(s.png.len() > 64, "{} is empty", s.id);
            assert_eq!(&s.png[..8], b"\x89PNG\r\n\x1a\n", "{} is not a PNG", s.id);
        }
    }

    #[test]
    fn there_are_eight_categories_and_all_are_used() {
        let cats = categories();
        assert_eq!(cats.len(), 8, "got {cats:?}");
        for c in &cats {
            assert!(
                CATALOG.iter().any(|s| s.category == *c),
                "category {c} has nothing in it"
            );
        }
    }
}

// ---- end of generated section ----
