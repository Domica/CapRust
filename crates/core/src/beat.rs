//! Beat grid estimation from mono PCM (spike).
//!
//! Pipeline: frame energies (256 samples, hop 64) -> spectral-flux-like
//! onset envelope -> peak-picked onsets -> BPM from inter-onset
//! interval histogram (60..240) -> grid from the first onset.
//!
//! Pure function over samples: no ffmpeg, no I/O, fully unit-testable
//! with synthetic click tracks. Shares the 8 kHz mono convention with
//! `waveform.rs` but deliberately does NOT reuse its peak buckets —
//! beat detection needs time resolution (~8 ms), buckets are far too
//! coarse.

use serde::{Deserialize, Serialize};

/// Detected beat grid. `bpm == 0.0` with empty `beats_ms` means no
/// usable onsets (silence / too few transients).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BeatGrid {
    pub bpm: f32,
    pub beats_ms: Vec<u64>,
}

/// Beat analysis stored per media item (see
/// `ProjectState::beat_grids`). `source_*` fingerprint the file the
/// grid was computed from; a mismatch means recompute.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BeatEntry {
    pub bpm: f32,
    pub beats_ms: Vec<u64>,
    pub source_len_bytes: u64,
    pub source_mtime_secs: u64,
}

const FRAME: usize = 256;
const HOP: usize = 64;
const MIN_GAP_SEC: f64 = 0.25; // fastest supported tempo: 240 BPM
const MIN_BPM: f64 = 60.0;
const MAX_BPM: f64 = 240.0;

pub fn detect_beats(samples: &[i16], sample_rate: u32) -> BeatGrid {
    let empty = BeatGrid {
        bpm: 0.0,
        beats_ms: Vec::new(),
    };
    if samples.is_empty() || sample_rate == 0 {
        return empty;
    }
    let sr = sample_rate as f64;
    let dur_sec = samples.len() as f64 / sr;

    // 1. Frame energies (mean square), hop-stepped.
    let mut energies: Vec<f64> = Vec::new();
    let mut i = 0;
    while i < samples.len() {
        let end = (i + FRAME).min(samples.len());
        let mut sum: u64 = 0;
        for &s in &samples[i..end] {
            let v = s as i64;
            sum += (v * v) as u64;
        }
        energies.push(sum as f64 / (end - i).max(1) as f64);
        if end == samples.len() {
            break;
        }
        i += HOP;
    }
    if energies.len() < 8 {
        return empty;
    }

    // 2. Flux = positive energy difference; onset = local max above
    // a relative threshold with a minimum gap.
    let mut flux = vec![0.0; energies.len()];
    for (k, f) in flux.iter_mut().enumerate() {
        let prev = if k == 0 { 0.0 } else { energies[k - 1] };
        *f = (energies[k] - prev).max(0.0);
    }
    let mean_flux: f64 = flux.iter().sum::<f64>() / flux.len() as f64;
    let thresh = mean_flux * 3.0 + 1e-3;
    let min_gap_frames = ((MIN_GAP_SEC * sr) / HOP as f64).round() as usize;
    let mut onsets: Vec<usize> = Vec::new();
    for k in 0..flux.len() {
        if flux[k] < thresh {
            continue;
        }
        let lo = k.saturating_sub(2);
        let hi = (k + 3).min(flux.len());
        if flux[lo..hi].iter().any(|&v| v > flux[k]) {
            continue;
        }
        if onsets.last().is_some_and(|&p| k - p < min_gap_frames) {
            continue;
        }
        onsets.push(k);
    }
    if onsets.len() < 4 {
        return empty;
    }

    // 3. BPM from the inter-onset interval histogram.
    let frame_sec = HOP as f64 / sr;
    let mut votes: std::collections::HashMap<i32, (usize, f64)> = Default::default();
    for w in onsets.windows(2) {
        let gap_sec = (w[1] - w[0]) as f64 * frame_sec;
        if gap_sec <= 0.0 {
            continue;
        }
        let bpm = 60.0 / gap_sec;
        if !(MIN_BPM..=MAX_BPM).contains(&bpm) {
            continue;
        }
        let bin = bpm.round() as i32;
        let e = votes.entry(bin).or_insert((0, 0.0));
        e.0 += 1;
        e.1 += gap_sec;
    }
    let (_, &(n, sum)) = votes.iter().max_by_key(|(_, (n, _))| *n).unwrap();
    if n < 2 {
        return empty;
    }
    let avg_gap = sum / n as f64;
    let bpm = (60.0 / avg_gap) as f32;

    // 4. Grid from the first onset.
    let t0 = onsets[0] as f64 * frame_sec;
    let step = 60.0 / bpm as f64;
    let mut beats_ms = Vec::new();
    let mut t = t0;
    while t <= dur_sec + 1e-6 {
        beats_ms.push((t * 1000.0).round() as u64);
        t += step;
    }
    BeatGrid { bpm, beats_ms }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 8000;

    /// Synthetic click track: short loud bursts every `period_sec`.
    fn click_track(dur_sec: f64, period_sec: f64) -> Vec<i16> {
        let n = (dur_sec * SR as f64) as usize;
        let mut s = vec![0i16; n];
        let mut t = 0.0;
        while t < dur_sec {
            let start = (t * SR as f64) as usize;
            for k in 0..48 {
                if start + k < n {
                    s[start + k] = 20_000;
                }
            }
            t += period_sec;
        }
        s
    }

    #[test]
    fn click_120bpm_detected() {
        let grid = detect_beats(&click_track(8.0, 0.5), SR);
        assert!((grid.bpm - 120.0).abs() < 2.0, "bpm {}", grid.bpm);
        assert!(
            (15..=17).contains(&grid.beats_ms.len()),
            "n={}",
            grid.beats_ms.len()
        );
        for w in grid.beats_ms.windows(2) {
            let gap = w[1] - w[0];
            assert!((480..=520).contains(&gap), "gap {gap}");
        }
    }

    #[test]
    fn click_90bpm_detected() {
        let grid = detect_beats(&click_track(6.0, 60.0 / 90.0), SR);
        assert!((grid.bpm - 90.0).abs() < 2.0, "bpm {}", grid.bpm);
        assert!(
            (8..=10).contains(&grid.beats_ms.len()),
            "n={}",
            grid.beats_ms.len()
        );
    }

    #[test]
    fn silence_gives_empty_grid() {
        let grid = detect_beats(&vec![0i16; 8000 * 4], SR);
        assert_eq!(grid.bpm, 0.0);
        assert!(grid.beats_ms.is_empty());
    }

    #[test]
    fn too_few_onsets_gives_empty_grid() {
        let mut s = vec![0i16; 8000 * 2];
        s[100] = i16::MAX;
        s[8000] = i16::MAX;
        let grid = detect_beats(&s, SR);
        assert!(grid.beats_ms.is_empty());
    }

    #[test]
    fn empty_input_gives_empty_grid() {
        let grid = detect_beats(&[], SR);
        assert_eq!(grid.bpm, 0.0);
    }
}
