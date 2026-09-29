//! Master-bus CLAP processing chain (Faza I, commit 3b-1).
//!
//! This commit actually loads each `.clap` binary and instantiates the
//! plugin via clack-host. Activation and audio processing come in 3b-2.
//!
//! Why split: loading exercises the host, the entry, and the descriptor
//! path end-to-end against a real binary (u-he ZebraHZ locally), and
//! catches ABI or link errors before we build the buffer plumbing.

use std::ffi::CString;

use anyhow::{anyhow, bail, Context, Result};
use clack_host::prelude::*;

use caprust_core::plugin::PluginInstance as PluginDescriptor;

// ── Host (MVP: no extensions, no async, no state) ───────────────────────

struct CapRustHostShared;

impl SharedHandler<'_> for CapRustHostShared {
    fn request_restart(&self) {}
    fn request_process(&self) {}
    fn request_callback(&self) {}
}

struct CapRustHost;

impl HostHandlers for CapRustHost {
    type Shared<'a> = CapRustHostShared;
    type MainThread<'a> = ();
    type AudioProcessor<'a> = ();

    fn declare_extensions(_builder: &mut HostExtensions<Self>, _shared: &Self::Shared<'_>) {
        // MVP: no extensions. Latency / params / logging are follow-ups.
    }
}

// ── Loaded plugin ───────────────────────────────────────────────────────

struct LoadedPlugin {
    /// The instance owns a clone of the CLAP entry internally, which
    /// keeps the underlying DLL/.SO mapped for the instance's lifetime.
    /// We do not need to hold the entry separately.
    _instance: PluginInstance<CapRustHost>,
}

impl LoadedPlugin {
    fn load(desc: &PluginDescriptor) -> Result<Self> {
        let entry = unsafe { PluginEntry::load(&desc.path) }
            .with_context(|| format!("load CLAP entry {}", desc.path.display()))?;

        let host_info = HostInfo::new(
            "CapRust",
            "CapRust",
            "https://github.com/Domica/CapRust",
            env!("CARGO_PKG_VERSION"),
        )
        .map_err(|e| anyhow!("HostInfo::new: {e:?}"))?;

        let plugin_id =
            CString::new(desc.plugin_id.as_str()).context("plugin id contains NUL byte")?;

        let instance = PluginInstance::<CapRustHost>::new(
            |_| CapRustHostShared,
            |_| (),
            &entry,
            &plugin_id,
            &host_info,
        )
        .map_err(|e| anyhow!("PluginInstance::new: {e:?}"))?;

        Ok(Self {
            _instance: instance,
        })
    }
}

// ── ClapChain ───────────────────────────────────────────────────────────

pub struct ClapChain {
    descriptors: Vec<PluginDescriptor>,
    plugins: Vec<LoadedPlugin>,
    sample_rate: u32,
    block_frames: usize,
}

impl ClapChain {
    /// Build a chain from persisted descriptors.
    ///
    /// Individual plugin load failures are logged and skipped so one
    /// broken `.clap` cannot silence a whole project.
    pub fn load(descriptors: Vec<PluginDescriptor>, sample_rate: u32, block_frames: usize) -> Self {
        let mut plugins = Vec::with_capacity(descriptors.len());
        for d in &descriptors {
            if d.bypassed {
                tracing::debug!("CLAP: skipping bypassed plugin {}", d.name);
                continue;
            }
            match LoadedPlugin::load(d) {
                Ok(p) => {
                    tracing::info!(
                        "CLAP: loaded {} ({}) from {}",
                        d.name,
                        d.plugin_id,
                        d.path.display(),
                    );
                    plugins.push(p);
                }
                Err(e) => {
                    tracing::warn!("CLAP: failed to load {} ({}): {e:#}", d.name, d.plugin_id,);
                }
            }
        }
        Self {
            descriptors,
            plugins,
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

    /// Number of plugins that actually loaded. Differs from `len()`
    /// when any plugin was bypassed or failed to load.
    pub fn loaded_len(&self) -> usize {
        self.plugins.len()
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn block_frames(&self) -> usize {
        self.block_frames
    }

    /// Process one block of interleaved stereo f32 samples in place.
    ///
    /// 3b-1: not implemented. Empty chain returns `Ok(())` (fast path);
    /// any loaded plugin causes `Err` so the reader loop logs once and
    /// continues with dry audio.
    pub fn process(&mut self, _interleaved: &mut [f32], _frames: usize) -> Result<()> {
        if self.plugins.is_empty() {
            return Ok(());
        }
        bail!(
            "CLAP chain processing not yet implemented ({} plugin(s) loaded, commit 3b-2)",
            self.plugins.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn empty_chain() -> ClapChain {
        ClapChain::load(vec![], 48_000, 1024)
    }

    fn one_bogus_plugin_chain() -> ClapChain {
        // A path that definitely does not exist; LoadedPlugin::load
        // will fail and the plugin will be skipped.
        let p = PluginDescriptor::new(
            "com.example.Missing",
            PathBuf::from("Z:/caprust_definitely_missing_xyz.clap"),
            "Missing",
        );
        ClapChain::load(vec![p], 48_000, 1024)
    }

    #[test]
    fn empty_chain_is_reported_empty() {
        let c = empty_chain();
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
        assert_eq!(c.loaded_len(), 0);
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
    fn bogus_plugin_is_skipped_not_panicked() {
        let c = one_bogus_plugin_chain();
        assert!(!c.is_empty());
        assert_eq!(c.len(), 1);
        assert_eq!(c.loaded_len(), 0, "bogus .clap must be skipped");
    }

    #[test]
    fn bypassed_plugin_is_skipped() {
        let mut p = PluginDescriptor::new(
            "com.example.Bypassed",
            PathBuf::from("Z:/caprust_bypassed.clap"),
            "Bypassed",
        );
        p.bypassed = true;
        let c = ClapChain::load(vec![p], 48_000, 1024);
        assert_eq!(c.len(), 1);
        assert_eq!(c.loaded_len(), 0);
    }

    #[test]
    fn accessors_round_trip() {
        let c = ClapChain::load(vec![], 44_100, 512);
        assert_eq!(c.sample_rate(), 44_100);
        assert_eq!(c.block_frames(), 512);
    }

    #[test]
    #[ignore = "requires ZebraHZ at a fixed path; run locally with --ignored"]
    fn loads_zebrahz_end_to_end() {
        let path = PathBuf::from(r"F:\Minimax H3\ZEBRA CLAP\ZebraHZ.clap");
        if !path.is_file() {
            eprintln!("skipping: ZebraHZ not at {}", path.display());
            return;
        }
        let desc = PluginDescriptor::new("com.u-he.ZebraHZ", path, "ZebraHZ");
        let c = ClapChain::load(vec![desc], 48_000, 1024);
        assert_eq!(c.len(), 1);
        assert_eq!(
            c.loaded_len(),
            1,
            "ZebraHZ should load; check stderr for the failure reason"
        );
    }
}
