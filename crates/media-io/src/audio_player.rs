//! Audio playback via cpal.
//!
//! **Phase H1**: sine-wave skeleton that proves cpal works in our environment.
//! **Phase H2** (current): `AudioPlayer::play_pcm_file()` — reads s16le stereo
//! PCM from a file through an SPSC ring buffer into the cpal callback.
//! **Phase H3** (next): wire `play_pcm_file` to the preview ffmpeg process,
//! which writes the temp PCM file (`-f s16le -ar 48000 -ac 2 <tmp>`).
//! **Phase H4**: audio-master playhead — UI reads `playhead_ms()` instead of
//! the wall-clock instant.
//!
//! Threading note: `cpal::Stream` is `!Send` on some platforms. Keep the
//! `AudioPlayer` on whichever thread created it (usually the UI thread).
//! The PCM reader is a separate thread; it exits on `stop_flag` or EOF.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::{HeapProd, HeapRb};

/// Sample rate we request from the device. Matches the ffmpeg PCM pipeline
/// planned for H3 (`-f s16le -ar 48000 -ac 2`).
pub const TARGET_SAMPLE_RATE: u32 = 48_000;
/// Channels we request from the device.
pub const TARGET_CHANNELS: u16 = 2;

/// Frames read per iteration from disk by the reader thread.
const READ_CHUNK_FRAMES: usize = 1024;
/// Ring buffer capacity in frames. Previously 9_600 (~200 ms at 48 kHz
/// stereo), which was not enough cushion: any time ffmpeg stopped writing
/// (because the UI stalled and backpressure rippled all the way up), the
/// ringbuf drained in 200 ms and the cpal callback wrote silence. Silence
/// was not counted in samples_played, so the playhead froze.
///
/// 96_000 frames gives 2 seconds of buffered audio, easily surviving the
/// short UI stalls that triggered the dropouts.
const RING_FRAMES: usize = 96_000;
/// Bytes per frame in the on-disk PCM stream (s16le stereo = 2 ch * 2 B).
const BYTES_PER_FRAME: u64 = 4;
/// Consecutive EOF reads before the reader gives up.
/// ffmpeg writes PCM sequentially from t=0 even when we asked to start at a
/// later offset (the -ss is only on the video output). The reader therefore
/// seeks to `start_ms * 192` bytes and has to wait until ffmpeg has DECODED
/// that many seconds of source and written them out. On a 15-second seek
/// this can take 10-20 real seconds. Give the reader a long enough window
/// (~20 s at 40 ms per retry) that it survives typical seeks, then still
/// exit cleanly if ffmpeg really did reach end-of-file.
const EOF_RETRIES: u32 = 500;
/// Sleep between EOF retries. Chosen so retries are cheap but the reader
/// stays responsive to the stop flag.
const EOF_SLEEP_MS: u64 = 40;

/// A sample source consumed by the cpal callback.
///
/// `fill(out)` writes as many f32 samples as it can, up to `out.len()`,
/// and returns the number actually written. Returning `0` means "exhausted"
/// and the caller pads the rest of the callback buffer with silence.
type SourceFn = Box<dyn FnMut(&mut [f32]) -> usize + Send>;

/// Owns a cpal output stream and exposes playback position to the UI.
pub struct AudioPlayer {
    /// Held so the stream stays alive. Dropping this struct stops playback.
    _stream: cpal::Stream,
    samples_played: Arc<AtomicU64>,
    sample_rate: u32,
    channels: u16,
    stop_flag: Arc<AtomicBool>,
    reader_handle: Option<JoinHandle<()>>,
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        // Signal the reader to exit, then wait for it so we don't leak threads.
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(h) = self.reader_handle.take() {
            let _ = h.join();
        }
    }
}

impl AudioPlayer {
    /// H1 diagnostic: play a continuous sine wave at `freq_hz`.
    ///
    /// Useful for verifying cpal works on a new machine without needing a
    /// PCM file. Not used by the UI in normal operation.
    pub fn play_sine(freq_hz: f32) -> Result<Self> {
        let source = make_sine_source(freq_hz);
        Self::spawn(source, None, "sine")
    }

    /// Play a raw PCM file (s16le, 48 kHz, stereo — the format ffmpeg will
    /// write in H3). `start_ms` seeks into the file before reading begins.
    ///
    /// Returns `Err` if no output device is available OR the file cannot be
    /// opened. The caller should handle that gracefully and keep the video
    /// preview running without sound.
    pub fn play_pcm_file(path: &Path, start_ms: u64) -> Result<Self> {
        // Verify the file exists before we commit to spawning anything.
        // NOTE: do NOT clamp byte_offset to the file's current size. At this
        // point ffmpeg may not have written anything yet (file size 0), and
        // clamping would reset the seek to 0 — so a call with start_ms=3300
        // would silently play from the very beginning of the source.
        std::fs::metadata(path)
            .with_context(|| format!("stat {}", path.display()))?;
        let byte_offset = pcm_seek_offset(start_ms, TARGET_SAMPLE_RATE);

        // SPSC ring buffer, f32 samples (interleaved stereo).
        let rb = HeapRb::<f32>::new(RING_FRAMES * TARGET_CHANNELS as usize);
        let (producer, mut consumer) = rb.split();

        let stop_flag = Arc::new(AtomicBool::new(false));
        let reader_handle =
            spawn_pcm_reader(path.to_path_buf(), byte_offset, producer, stop_flag.clone())?;

        let source: SourceFn = Box::new(move |out: &mut [f32]| consumer.pop_slice(out));

        Self::spawn(source, Some((stop_flag, reader_handle)), "pcm")
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

    /// Shared device/stream setup. `reader` is `None` for sources that don't
    /// need a background thread (sine), `Some((flag, handle))` for PCM.
    fn spawn(
        source: SourceFn,
        reader: Option<(Arc<AtomicBool>, JoinHandle<()>)>,
        tag: &str,
    ) -> Result<Self> {
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
            "cpal[{}]: device={} fmt={:?} rate={} ch={}",
            tag,
            device_name,
            sample_format,
            sample_rate,
            channels
        );

        let stream = match sample_format {
            SampleFormat::F32 => {
                build_stream::<f32>(&device, &config, source, &samples_played)?
            }
            SampleFormat::I16 => {
                build_stream::<i16>(&device, &config, source, &samples_played)?
            }
            SampleFormat::U16 => {
                build_stream::<u16>(&device, &config, source, &samples_played)?
            }
            other => return Err(anyhow!("unsupported sample format: {other:?}")),
        };

        stream.play().context("cpal stream.play()")?;

        let (stop_flag, reader_handle) = match reader {
            Some((flag, handle)) => (flag, Some(handle)),
            None => (Arc::new(AtomicBool::new(false)), None),
        };

        Ok(Self {
            _stream: stream,
            samples_played,
            sample_rate,
            channels,
            stop_flag,
            reader_handle,
        })
    }
}

// ---------------------------------------------------------------------------
// Stream construction
// ---------------------------------------------------------------------------

/// Build a generic cpal output stream fed by `source`.
///
/// Underrun policy: if `source` returns fewer samples than requested,
/// the callback pads the remainder with silence. Keeps the audio device
/// alive (avoids clicks/stutters) at the cost of a brief gap.
fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut source: SourceFn,
    samples_played: &Arc<AtomicU64>,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = config.channels as usize;
    let counter = samples_played.clone();
    let mut scratch: Vec<f32> = vec![0.0; 8192];

    device
        .build_output_stream(
            config,
            move |data: &mut [T], _info: &cpal::OutputCallbackInfo| {
                let need = data.len();
                let mut written = 0usize;
                // Frames of *real* audio pulled from the source this callback.
                // Silence written to fill an underrun is NOT counted, because
                // otherwise the playhead would advance during the window where
                // the PCM reader hasn't produced data yet (e.g. ffmpeg still
                // buffering) — making the UI believe audio has been playing
                // when the listener has heard nothing.
                let mut real_samples_written = 0usize;

                while written < need {
                    let want = (need - written).min(scratch.len());
                    let got = source(&mut scratch[..want]);

                    if got == 0 {
                        // Source exhausted — silence the rest of this buffer.
                        for s in &mut data[written..] {
                            *s = T::from_sample(0.0_f32);
                        }
                        break;
                    }

                    for i in 0..got {
                        data[written + i] = T::from_sample(scratch[i]);
                    }
                    written += got;
                    real_samples_written += got;

                    if got < want {
                        // Partial fill = underrun. Pad with silence.
                        for s in &mut data[written..] {
                            *s = T::from_sample(0.0_f32);
                        }
                        break;
                    }
                }

                // Count only frames that came from real source data.
                counter.fetch_add(
                    (real_samples_written / channels.max(1)) as u64,
                    Ordering::Relaxed,
                );
            },
            |err| tracing::error!("cpal stream error: {err}"),
            None,
        )
        .context("cpal build_output_stream")
}

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

/// Sine source used by `play_sine`. Never exhausts — always fills `out`.
fn make_sine_source(freq_hz: f32) -> SourceFn {
    use std::f32::consts::PI;

    let phase_inc = 2.0 * PI * freq_hz / TARGET_SAMPLE_RATE as f32;
    let mut phase: f32 = 0.0;

    Box::new(move |out: &mut [f32]| {
        for s in out.iter_mut() {
            *s = phase.sin() * 0.15; // low volume for a test tone
            phase += phase_inc;
            if phase > 2.0 * PI {
                phase -= 2.0 * PI;
            }
        }
        out.len()
    })
}

// ---------------------------------------------------------------------------
// PCM reader
// ---------------------------------------------------------------------------

/// Byte offset into a stereo s16le stream for `start_ms`.
///
/// Pure math — no I/O. Formula: ms * samples_per_ms * bytes_per_frame.
/// `samples_per_ms = sample_rate / 1000`. For 48 kHz stereo s16le: 192 B/ms.
pub(crate) fn pcm_seek_offset(start_ms: u64, sample_rate: u32) -> u64 {
    start_ms * sample_rate as u64 * BYTES_PER_FRAME / 1000
}

/// Spawn the reader thread. Fails only if the OS refuses the thread.
fn spawn_pcm_reader(
    path: PathBuf,
    byte_offset: u64,
    mut producer: HeapProd<f32>,
    stop: Arc<AtomicBool>,
) -> Result<JoinHandle<()>> {
    let handle = thread::Builder::new()
        .name("caprust-audio-reader".into())
        .spawn(move || {
            if let Err(e) = pcm_reader_loop(&path, byte_offset, &mut producer, &stop) {
                tracing::warn!("audio reader exited: {e:#}");
            } else {
                tracing::debug!("audio reader finished");
            }
        })
        .context("spawn audio reader thread")?;

    Ok(handle)
}

/// Read s16le stereo, convert to f32, push into the ring buffer. Exits on
/// `stop_flag`, EOF, or I/O error.
fn pcm_reader_loop(
    path: &Path,
    byte_offset: u64,
    producer: &mut HeapProd<f32>,
    stop: &Arc<AtomicBool>,
) -> Result<()> {
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    file.seek(SeekFrom::Start(byte_offset))
        .context("seek pcm file")?;

    let mut bytes = vec![0u8; READ_CHUNK_FRAMES * BYTES_PER_FRAME as usize];
    let mut eof_streak: u32 = 0;
    let mut first_data = true;

    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }

        let n = file.read(&mut bytes).context("read pcm")?;
        if n == 0 {
            eof_streak += 1;
            if eof_streak >= EOF_RETRIES {
                tracing::debug!("pcm reader: EOF after {eof_streak} retries");
                return Ok(());
            }
            thread::sleep(Duration::from_millis(EOF_SLEEP_MS));
            continue;
        }
        eof_streak = 0;
        if first_data {
            tracing::info!(
                "pcm reader: first real data after {} EOF polls (offset {} bytes)",
                eof_streak,
                byte_offset
            );
            first_data = false;
        }

        // s16le → f32 in [-1.0, 1.0). `chunks_exact(2)` discards an odd
        // trailing byte — can't happen with well-formed PCM but be safe.
        let samples: Vec<f32> = bytes[..n]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect();

        let mut pushed = 0usize;
        while pushed < samples.len() {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            let k = producer.push_slice(&samples[pushed..]);
            if k == 0 {
                // Ring full — wait for the cpal callback to drain it.
                thread::sleep(Duration::from_millis(5));
            } else {
                pushed += k;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Requires a working audio device. Run manually with:
    ///     cargo nextest run -p caprust-media-io --run-ignored only audio_smoke
    #[test]
    #[ignore = "requires an audio output device"]
    fn audio_smoke_plays_sine_briefly() -> Result<()> {
        let player = AudioPlayer::play_sine(440.0)?;
        std::thread::sleep(Duration::from_millis(200));
        let pos = player.playhead_ms();
        assert!(pos > 0, "expected some samples to have played, got {pos} ms");
        Ok(())
    }

    #[test]
    fn playhead_ms_derives_from_sample_counter() {
        let rate = 48_000u32;
        let frames = 48_000u64;
        let ms = (frames * 1000) / rate as u64;
        assert_eq!(ms, 1000);
    }

    #[test]
    fn pcm_seek_offset_math() {
        // 48 kHz stereo s16le = 192 000 B/s
        assert_eq!(pcm_seek_offset(0, 48_000), 0);
        assert_eq!(pcm_seek_offset(1000, 48_000), 192_000);
        assert_eq!(pcm_seek_offset(500, 48_000), 96_000);
        assert_eq!(pcm_seek_offset(1, 48_000), 192);
    }

    #[test]
    fn pcm_seek_offset_monotonic() {
        // Sanity: larger ms → larger offset, never wraps for realistic values.
        let mut prev = 0u64;
        for ms in (0..60_000).step_by(250) {
            let off = pcm_seek_offset(ms, 48_000);
            assert!(off >= prev, "offset not monotonic at {ms} ms");
            prev = off;
        }
    }
}
