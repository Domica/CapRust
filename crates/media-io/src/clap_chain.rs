//! Master-bus CLAP processing chain (Faza I, commit 3a).
//!
//! This module owns the *plumbing* that connects the audio reader
//! thread to a per-project master plugin chain. The actual CLAP
//! processing lives in `LoadedPlugin` and is filled in by commit 3b.
//!
//! Design notes:
//! - The chain runs on the PCM reader thread (`spawn_pcm_reader`),
//!   which reads 1024-frame chunks of interleaved stereo f32. That
//!   matches our CLAP block size.
//! - An empty chain short-circuits to a no-op so projects with no
//!   plugins keep the existing zero-overhead path.
//! - Feature-gated behind `clap` so Linux CI without the SDK still
//!   builds (DIRECTIVES 10.6).

use anyhow::{bail, Result};

use caprust_core::plugin::PluginInstance;

/// A loaded master chain. Lives on the reader thread.
pub struct ClapChain {
    /// Saved descriptors that we were asked to load. Commit 3a stores
    /// them verbatim; commit 3b replaces this with activated instances.
    descriptors: Vec<PluginInstance>,
    sample_rate: u32,
    block_frames: usize,
}

impl ClapChain {
    /// Build a chain from persisted `PluginInstance` records.
    ///
    /// Commit 3a does not yet instantiate plugins; it only retains the
    /// descriptors so the reader loop can call `process()` through the
    /// same code path it will use in 3b.
    pub fn load(descriptors: Vec<PluginInstance>, sample_rate: u32, block_frames: usize) -> Self {
        Self {
            descriptors,
            sample_rate,
            block_frames,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.descriptors.is_empty()
    }

    pub fn len(&self) -> usize {
        self.descriptors.len()
    }

    /// Process one block of interleaved stereo f32 samples in place.
    ///
    /// Empty chain: returns `Ok(())` immediately (fast path).
    /// Non-empty chain: commit 3a returns `Err` so the reader loop
    /// logs once and continues with dry audio rather than corrupting
    /// the stream. Commit 3b replaces this with real processing.
    pub fn process(&mut self, _interleaved: &mut [f32], _frames: usize) -> Result<()> {
        if self.is_empty() {
            return Ok(());
        }
        bail!(
            "CLAP chain not yet implemented ({} plugin(s) pending, commit 3b)",
            self.descriptors.len()
        );
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn block_frames(&self) -> usize {
        self.block_frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn empty_chain() -> ClapChain {
        ClapChain::load(vec![], 48_000, 1024)
    }

    fn one_plugin_chain() -> ClapChain {
        let p = PluginInstance::new("x", PathBuf::from("/x.clap"), "X");
        ClapChain::load(vec![p], 48_000, 1024)
    }

    #[test]
    fn empty_chain_is_reported_empty() {
        let c = empty_chain();
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn empty_chain_process_is_noop() {
        let mut c = empty_chain();
        let mut buf = vec![0.5f32; 2048];
        let before = buf.clone();
        c.process(&mut buf, 1024).unwrap();
        assert_eq!(buf, before, "empty chain must not modify samples");
    }

    #[test]
    fn nonempty_chain_reports_len() {
        let c = one_plugin_chain();
        assert!(!c.is_empty());
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn nonempty_chain_process_errors_in_3a() {
        let mut c = one_plugin_chain();
        let mut buf = vec![0.0f32; 2048];
        let err = c.process(&mut buf, 1024).unwrap_err();
        assert!(err.to_string().contains("commit 3b"));
    }

    #[test]
    fn accessors_round_trip() {
        let c = ClapChain::load(vec![], 44_100, 512);
        assert_eq!(c.sample_rate(), 44_100);
        assert_eq!(c.block_frames(), 512);
    }
}
