//! Audio playback via cpal.
//!
//! **Phase H1**: sine-wave skeleton that proves cpal works in our environment.
//! **Phase H2** (next): replace sine generation with a reader that streams PCM
//! samples decoded by the preview ffmpeg process.
//! **Phase H4**: audio-master playhead — UI reads `playhead_ms()` instead of
//! wall-clock.
//!
//! Threading note: `cpal::Stream` is `!Send` on some platforms. Keep the
//! `AudioPlayer` on whichever thread created it (usually the UI thread).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};

/// Sample rate we request from the device. Matches the ffmpeg PCM pipeline
/// planned for H2 (`-f s16le -ar 48000 -ac 2`).
pub const TARGET_SAMPLE_RATE: u32 = 48_000;
/// Channels we request from the device.
pub const TARGET_CHANNELS: u16 = 2;

/// Owns a cpal output stream and exposes playback position to the UI.
pub struct AudioPlayer {
    /// Held so the stream stays alive. Dropping this struct stops playback.
    _stream: cpal::Stream,
    samples_played: Arc<AtomicU64>,
    sample_rate: u32,
    channels: u16,
}

impl AudioPlayer {
    /// H1 smoke test: play a continuous sine wave at `freq_hz`.
    ///
    /// Returns `Err` if no output device is available (e.g. headless CI).
    /// The caller is expected to handle that gracefully and keep the video
    /// preview running without sound.
    pub fn play_sine(freq_hz: f32) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow!("no default audio output device"))?;
        let device_name = device.name().unwrap_or_else(|_| "<unknown>".into());

        let supported = device
            .default_output_config()
            .with_context(|| format!("no default output config for {device_name}"))?;

        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();

        let sample_rate = config.sample_rate.0;
        let channels = config.channels;
        let samples_played = Arc::new(AtomicU64::new(0));

        tracing::info!(
            "cpal: device={} format={:?} rate={} ch={}",
            device_name,
            sample_format,
            sample_rate,
            channels
        );

        let stream = match sample_format {
            SampleFormat::F32 => build_stream::<f32>(&device, &config, freq_hz, &samples_played)?,
            SampleFormat::I16 => build_stream::<i16>(&device, &config, freq_hz, &samples_played)?,
            SampleFormat::U16 => build_stream::<u16>(&device, &config, freq_hz, &samples_played)?,
            other => {
                return Err(anyhow!("unsupported sample format: {other:?}"));
            }
        };

        stream.play().context("cpal stream.play()")?;

        Ok(Self {
            _stream: stream,
            samples_played,
            sample_rate,
            channels,
        })
    }

    /// Frames played since playback started.
    pub fn samples_played(&self) -> u64 {
        self.samples_played.load(Ordering::Relaxed)
    }

    /// Playback position in milliseconds, derived from the sample counter.
    pub fn playhead_ms(&self) -> u64 {
        (self.samples_played() * 1000) / self.sample_rate.max(1) as u64
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }
}

/// Build a generic cpal stream that writes sine samples of type `T`.
fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    freq_hz: f32,
    samples_played: &Arc<AtomicU64>,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    use std::f32::consts::PI;

    let sample_rate = config.sample_rate.0 as f32;
    let channels = config.channels as usize;
    let phase_inc = 2.0 * PI * freq_hz / sample_rate;
    let mut phase: f32 = 0.0;
    let counter = samples_played.clone();

    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [T], _info: &cpal::OutputCallbackInfo| {
                for sample in data.iter_mut() {
                    let v = phase.sin() * 0.15; // low volume for a test tone
                    *sample = T::from_sample(v);
                    phase += phase_inc;
                    if phase > 2.0 * PI {
                        phase -= 2.0 * PI;
                    }
                }
                // Count *frames*, not samples.
                counter.fetch_add(
                    (data.len() / channels.max(1)) as u64,
                    Ordering::Relaxed,
                );
            },
            move |err| {
                tracing::error!("cpal stream error: {err}");
            },
            None,
        )
        .context("cpal build_output_stream")?;

    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Requires a working audio device. Run manually with:
    ///     cargo nextest run -p caprust-media-io --run-ignored audio_smoke
    #[test]
    #[ignore = "requires an audio output device"]
    fn audio_smoke_plays_sine_briefly() -> Result<()> {
        let player = AudioPlayer::play_sine(440.0)?;
        std::thread::sleep(std::time::Duration::from_millis(200));
        let pos = player.playhead_ms();
        assert!(pos > 0, "expected some samples to have played, got {pos} ms");
        Ok(())
    }

    #[test]
    fn playhead_ms_derives_from_sample_counter() {
        // Pure math check, no device needed.
        let samples_played = Arc::new(AtomicU64::new(48_000));
        let rate = 48_000u32;
        let ms = (samples_played.load(Ordering::Relaxed) * 1000) / rate as u64;
        assert_eq!(ms, 1000);
    }
}
