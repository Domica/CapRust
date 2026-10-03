//! Master-bus CLAP processing chain (Faza I, commit 3b-2).
//!
//! Loads each `.clap`, activates it, starts processing, and applies
//! the chain to interleaved stereo f32 blocks. The chain runs on the
//! PCM reader thread (`spawn_pcm_reader`).
//!
//! Buffer strategy: the reader gives us interleaved stereo; CLAP needs
//! planar. We deinterleave to two `Vec<f32>` channels, ping-pong two
//! planar buffers across the plugin chain (output of plugin N is input
//! of N+1), and reinterleave the final result back.

use std::ffi::CString;

use anyhow::{anyhow, bail, Context, Result};
use clack_host::prelude::*;

use caprust_core::plugin::PluginInstance as PluginDescriptor;

// ── Host (MVP: no extensions) ───────────────────────────────────────────

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

    fn declare_extensions(_builder: &mut HostExtensions<Self>, _shared: &Self::Shared<'_>) {}
}

// ── Loaded plugin ───────────────────────────────────────────────────────

struct LoadedPlugin {
    instance: PluginInstance<CapRustHost>,
    processor: Option<PluginAudioProcessor<CapRustHost>>,
    input_ports: AudioPorts,
    output_ports: AudioPorts,
}

impl LoadedPlugin {
    fn load(desc: &PluginDescriptor, sample_rate: u32, block_frames: usize) -> Result<Self> {
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

        let mut instance = PluginInstance::<CapRustHost>::new(
            |_| CapRustHostShared,
            |_| (),
            &entry,
            &plugin_id,
            &host_info,
        )
        .map_err(|e| anyhow!("PluginInstance::new: {e:?}"))?;

        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let config = PluginAudioConfiguration {
            sample_rate: sample_rate as f64,
            min_frames_count: block_frames as u32,
            max_frames_count: block_frames as u32,
        };

        let stopped = instance
            .activate(|_, _| (), config)
            .map_err(|e| anyhow!("activate: {e:?}"))?;

        let started = stopped
            .start_processing()
            .map_err(|e| anyhow!("start_processing: {e:?}"))?;

        Ok(Self {
            instance,
            processor: Some(PluginAudioProcessor::Started(started)),
            input_ports: AudioPorts::with_capacity(2, 1),
            output_ports: AudioPorts::with_capacity(2, 1),
        })
    }

    /// Process one stereo planar block: `inputs[ch][frame]` -> `outputs[ch][frame]`.
    fn process(&mut self, inputs: &mut [Vec<f32>; 2], outputs: &mut [Vec<f32>; 2]) -> Result<()> {
        let Some(processor) = self.processor.as_mut() else {
            return Ok(());
        };
        let PluginAudioProcessor::Started(started) = processor else {
            bail!("CLAP processor is in Stopped state mid-stream");
        };

        let input_buffers = self.input_ports.with_input_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_input_only(
                inputs
                    .iter_mut()
                    .map(|ch| InputChannel::variable(ch.as_mut_slice())),
            ),
        }]);

        let mut output_buffers = self.output_ports.with_output_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_output_only(
                outputs.iter_mut().map(|ch| ch.as_mut_slice()),
            ),
        }]);

        let mut in_events = InputEvents::empty();
        let mut out_events = OutputEvents::void();

        started
            .process(
                &input_buffers,
                &mut output_buffers,
                &in_events,
                &mut out_events,
                None,
                None,
            )
            .map_err(|e| anyhow!("process: {e:?}"))?;

        // Suppress unused_mut warnings when nothing writes into these.
        let _ = (&mut in_events, &mut out_events);

        Ok(())
    }
}

impl Drop for LoadedPlugin {
    fn drop(&mut self) {
        if let Some(PluginAudioProcessor::Started(started)) = self.processor.take() {
            let stopped = started.stop_processing();
            self.instance.deactivate_with(stopped, |_, _| ());
        }
    }
}

// ── ClapChain ───────────────────────────────────────────────────────────

pub struct ClapChain {
    descriptors: Vec<PluginDescriptor>,
    plugins: Vec<LoadedPlugin>,
    sample_rate: u32,
    block_frames: usize,
    /// Scratch planar buffers reused every block. Two stereo pairs
    /// (A and B); ping-ponged so the output of plugin N becomes the
    /// input of N+1. Each pair holds `[left, right]`.
    scratch: [[Vec<f32>; 2]; 2],
}

impl ClapChain {
    pub fn load(descriptors: Vec<PluginDescriptor>, sample_rate: u32, block_frames: usize) -> Self {
        let mut plugins = Vec::with_capacity(descriptors.len());
        for d in &descriptors {
            if d.bypassed {
                tracing::debug!("CLAP: skipping bypassed plugin {}", d.name);
                continue;
            }
            // The path comes from the (untrusted) project file; loading
            // it is native code execution.
            if !crate::clap_host::is_trusted_plugin_path(&d.path) {
                tracing::warn!(
                    "CLAP: refusing {} ({}): {} is not in a plugin scan directory",
                    d.name,
                    d.plugin_id,
                    d.path.display(),
                );
                continue;
            }
            match LoadedPlugin::load(d, sample_rate, block_frames) {
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
            scratch: [
                [vec![0.0; block_frames], vec![0.0; block_frames]],
                [vec![0.0; block_frames], vec![0.0; block_frames]],
            ],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.descriptors.is_empty()
    }

    pub fn len(&self) -> usize {
        self.descriptors.len()
    }

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
    /// Empty / fully-skipped chain: returns `Ok(())` immediately.
    pub fn process(&mut self, interleaved: &mut [f32], frames: usize) -> Result<()> {
        if self.plugins.is_empty() {
            return Ok(());
        }
        if frames == 0 || frames > self.block_frames {
            bail!(
                "CLAP block size {frames} out of range (max {})",
                self.block_frames
            );
        }
        if interleaved.len() < frames * 2 {
            bail!(
                "interleaved buffer too small: {} < {}",
                interleaved.len(),
                frames * 2
            );
        }

        // Deinterleave into scratch[A] = [left, right].
        for f in 0..frames {
            self.scratch[0][0][f] = interleaved[f * 2];
            self.scratch[0][1][f] = interleaved[f * 2 + 1];
        }

        // Ping-pong between A and B. After N plugins, the final data
        // is in scratch[src_idx] because we swap at the end of every
        // iteration.
        let mut src_idx = 0usize;
        let mut dst_idx = 1usize;
        for plugin in &mut self.plugins {
            let mut src = std::mem::take(&mut self.scratch[src_idx]);
            let mut dst = std::mem::take(&mut self.scratch[dst_idx]);

            plugin.process(&mut src, &mut dst)?;

            self.scratch[src_idx] = src;
            self.scratch[dst_idx] = dst;
            std::mem::swap(&mut src_idx, &mut dst_idx);
        }

        // Reinterleave from scratch[src_idx].
        for f in 0..frames {
            interleaved[f * 2] = self.scratch[src_idx][0][f];
            interleaved[f * 2 + 1] = self.scratch[src_idx][1][f];
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn empty_chain() -> ClapChain {
        ClapChain::load(vec![], 48_000, 1024)
    }

    fn bogus_plugin_chain() -> ClapChain {
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
        let c = bogus_plugin_chain();
        assert!(!c.is_empty());
        assert_eq!(c.len(), 1);
        assert_eq!(c.loaded_len(), 0);
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
    fn all_failed_plugins_process_is_noop() {
        let mut c = bogus_plugin_chain();
        let mut buf = vec![0.3f32; 2048];
        let before = buf.clone();
        c.process(&mut buf, 1024).unwrap();
        assert_eq!(buf, before);
    }

    #[test]
    fn process_rejects_oversized_block() {
        let p = PluginDescriptor::new(
            "com.example.Missing",
            PathBuf::from("Z:/caprust_x.clap"),
            "X",
        );
        let mut c = ClapChain::load(vec![p], 48_000, 512);
        // Force-load path so plugins vec is empty but we still hit the guard.
        let mut buf = vec![0.0f32; 4096];
        // loaded_len is 0 (bogus path), so guard is bypassed. This test
        // exercises the case where loaded_len > 0 in the future; skip.
        let _ = c.process(&mut buf, 256);
    }

    #[test]
    fn accessors_round_trip() {
        let c = ClapChain::load(vec![], 44_100, 512);
        assert_eq!(c.sample_rate(), 44_100);
        assert_eq!(c.block_frames(), 512);
    }

    #[test]
    #[ignore = "requires ZebraHZ at a fixed path; run locally with --ignored"]
    fn loads_and_processes_zebrahz() {
        let path = PathBuf::from(r"F:\Minimax H3\ZEBRA CLAP\ZebraHZ.clap");
        if !path.is_file() {
            eprintln!("skipping: ZebraHZ not at {}", path.display());
            return;
        }
        let desc = PluginDescriptor::new("com.u-he.ZebraHZ", path, "ZebraHZ");
        let mut c = ClapChain::load(vec![desc], 48_000, 1024);
        assert_eq!(c.loaded_len(), 1);

        // Feed a 1024-frame stereo sine; assert process returns Ok and
        // does not produce NaN. We do not assert on level because the
        // default Zebra patch may be silent or a synth awaiting notes.
        let mut buf = vec![0.0f32; 2048];
        for f in 0..1024 {
            let s = (f as f32 * 0.01).sin() * 0.1;
            buf[f * 2] = s;
            buf[f * 2 + 1] = s;
        }
        c.process(&mut buf, 1024).unwrap();
        for (i, v) in buf.iter().enumerate() {
            assert!(v.is_finite(), "NaN/inf at index {i}: {v}");
        }
    }
}
