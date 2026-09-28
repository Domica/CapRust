//! Text translation via the MyMemory public API.
//!
//! MyMemory (https://mymemory.translated.net) is free, requires no
//! account, and supports Croatian. Downsides: per-segment HTTP calls
//! (no batch endpoint), a soft daily quota, and translations that are
//! less polished than DeepL. The tradeoff is deliberate: it lets
//! Captions translation ship without asking the user for an API key
//! or a credit card.
//!
//! A local Marian NMT backend (`Helsinki-NLP/opus-mt-<src>-<tgt>`
//! through tract) is on the roadmap as Phase 2 (§32). The
//! `TranslateProvider` trait below is the seam: swapping in a new
//! backend should not touch any caller.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

/// Maximum byte length MyMemory accepts in the `q` parameter. Anything
/// longer is truncated client-side and the response is unreliable, so
/// we skip such segments and report them in the progress stream.
pub const MAX_QUERY_BYTES: usize = 500;

/// MyMemory's public endpoint. No auth, no API key.
pub const MYMEMORY_ENDPOINT: &str = "https://api.mymemory.translated.net/get";

/// Small delay between per-segment requests so a long caption batch
/// does not trip the anonymous rate limit. MyMemory tolerates roughly
/// 10 req/s on the anonymous tier; 150 ms is comfortable.
const REQUEST_SPACING: Duration = Duration::from_millis(150);

/// Progress events emitted by the background translation thread.
#[derive(Debug, Clone)]
pub enum TranslateEvent {
    /// `done` of `total` segments processed. Emitted after each HTTP
    /// round-trip completes (or the segment is skipped).
    Progress { done: usize, total: usize },
    /// All segments processed. `translations[i]` corresponds to the
    /// i-th input text. Untranslatable segments (too long, failed)
    /// carry the original text so ordering is preserved.
    Done {
        translations: Vec<String>,
        skipped: Vec<usize>,
    },
    /// Fatal error — nothing was translated.
    Failed(String),
}

/// One translation request. `source` may be `"auto"` (the request will
/// be sent with an empty language hint; MyMemory will guess), or a
/// two-letter code.
#[derive(Debug, Clone)]
pub struct TranslateRequest {
    pub texts: Vec<String>,
    pub source: String,
    pub target: String,
    /// Optional contact email. MyMemory raises the anonymous quota from
    /// ~5k words/day to ~50k words/day when provided. None = anonymous.
    pub email: Option<String>,
}

/// Decode the subset of MyMemory's JSON we care about.
///
/// Response shape on success:
/// ```json
/// { "responseData": { "translatedText": "..." },
///   "responseStatus": 200,
///   "responseDetails": "" }
/// ```
///
/// On quota exceeded the shape stays the same but `responseStatus`
/// is non-200 or `responseDetails` contains a warning; the caller must
/// treat both as failure.
fn parse_response(body: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct ResponseData {
        #[serde(rename = "translatedText")]
        translated_text: String,
    }
    #[derive(Deserialize)]
    struct Response {
        #[serde(rename = "responseData")]
        data: ResponseData,
        #[serde(rename = "responseStatus")]
        status: serde_json::Value,
        #[serde(rename = "responseDetails", default)]
        details: String,
    }

    let parsed: Response = serde_json::from_str(body).context("parse MyMemory JSON")?;

    // `responseStatus` comes back as a number (200) on success and can
    // be a string ("403") on some quota failures. Accept either.
    let code = match &parsed.status {
        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0),
        serde_json::Value::String(s) => s.parse().unwrap_or(0),
        _ => 0,
    };
    if code != 200 {
        return Err(anyhow!("MyMemory status {code}: {}", parsed.details));
    }
    // Defence in depth: quota warnings sometimes come back with status
    // 200 but a warning string in `translatedText`.
    if parsed.data.translated_text.starts_with("MYMEMORY WARNING") {
        return Err(anyhow!(
            "MyMemory quota warning: {}",
            parsed.data.translated_text
        ));
    }
    Ok(parsed.data.translated_text)
}

/// URL-encode a value for a query string. We only use `ureq`'s own
/// `query` builder here, so this is used solely for building the
/// `langpair` value.
fn langpair(source: &str, target: &str) -> String {
    // "auto" is our UI-level placeholder. MyMemory does not implement
    // detection the same way DeepL does; the pair with "auto" is
    // accepted and the service picks a source it thinks fits.
    format!("{source}|{target}")
}

/// Send one translation request. Blocking, runs on the calling thread.
fn translate_one(text: &str, source: &str, target: &str, email: Option<&str>) -> Result<String> {
    if text.len() > MAX_QUERY_BYTES {
        return Err(anyhow!(
            "segment exceeds {MAX_QUERY_BYTES} bytes ({} given)",
            text.len()
        ));
    }
    let mut req = ureq::get(MYMEMORY_ENDPOINT)
        .query("q", text)
        .query("langpair", &langpair(source, target));
    if let Some(addr) = email {
        if !addr.trim().is_empty() {
            req = req.query("de", addr);
        }
    }
    let resp = req.call().with_context(|| {
        format!(
            "GET {MYMEMORY_ENDPOINT} ({} -> {target})",
            langpair(source, target)
        )
    })?;
    let body = resp.into_string().context("read MyMemory body")?;
    parse_response(&body)
}

/// Spawn a background thread that translates `req.texts` one segment
/// at a time, emitting `Progress` after each. Returns a receiver the
/// UI polls.
pub fn spawn_translate(req: TranslateRequest) -> Receiver<TranslateEvent> {
    let (tx, rx) = channel();
    thread::spawn(move || run_translate(req, &tx));
    rx
}

fn run_translate(req: TranslateRequest, tx: &Sender<TranslateEvent>) {
    let total = req.texts.len();
    let mut out: Vec<String> = Vec::with_capacity(total);
    let mut skipped: Vec<usize> = Vec::new();

    for (i, text) in req.texts.iter().enumerate() {
        match translate_one(text, &req.source, &req.target, req.email.as_deref()) {
            Ok(translated) => out.push(translated),
            Err(e) => {
                tracing::warn!("translate: segment {i} ({:?}) failed: {e}", preview(text));
                out.push(text.clone());
                skipped.push(i);
            }
        }
        let _ = tx.send(TranslateEvent::Progress { done: i + 1, total });
        if i + 1 < total {
            thread::sleep(REQUEST_SPACING);
        }
    }

    let _ = tx.send(TranslateEvent::Done {
        translations: out,
        skipped,
    });
}

/// Short preview of a text for logs: first 40 chars, ellipsis if longer.
fn preview(text: &str) -> String {
    let mut s: String = text.chars().take(40).collect();
    if text.chars().count() > 40 {
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_success_response() {
        let body = r#"{
            "responseData": { "translatedText": "Bok svijete" },
            "responseStatus": 200,
            "responseDetails": ""
        }"#;
        assert_eq!(parse_response(body).unwrap(), "Bok svijete");
    }

    #[test]
    fn parse_accepts_string_status() {
        let body = r#"{
            "responseData": { "translatedText": "Bok" },
            "responseStatus": "200",
            "responseDetails": ""
        }"#;
        assert_eq!(parse_response(body).unwrap(), "Bok");
    }

    #[test]
    fn parse_rejects_non_200_status() {
        let body = r#"{
            "responseData": { "translatedText": "" },
            "responseStatus": 403,
            "responseDetails": "QUERY LENGTH LIMIT EXCEEDED"
        }"#;
        let err = parse_response(body).unwrap_err().to_string();
        assert!(err.contains("403"), "{err}");
        assert!(err.contains("QUERY LENGTH"), "{err}");
    }

    #[test]
    fn parse_rejects_quota_warning_in_text() {
        let body = r#"{
            "responseData": { "translatedText": "MYMEMORY WARNING: YOU USED ALL AVAILABLE FREE TRANSLATIONS FOR TODAY" },
            "responseStatus": 200,
            "responseDetails": ""
        }"#;
        let err = parse_response(body).unwrap_err().to_string();
        assert!(err.contains("quota warning"), "{err}");
    }

    #[test]
    fn parse_rejects_malformed_json() {
        assert!(parse_response("not json").is_err());
    }

    #[test]
    fn langpair_joins_source_and_target() {
        assert_eq!(langpair("en", "hr"), "en|hr");
        assert_eq!(langpair("auto", "hr"), "auto|hr");
    }

    #[test]
    fn preview_truncates_long_text() {
        let long = "a".repeat(60);
        let p = preview(&long);
        assert_eq!(p.chars().count(), 41, "{p}");
        assert!(p.ends_with('…'));
    }

    #[test]
    fn preview_short_text_unchanged() {
        assert_eq!(preview("hello"), "hello");
    }
}
