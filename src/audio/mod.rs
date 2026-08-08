//! One-shot background audio analysis for the Now Playing waveform.
//!
//! Playback itself is entirely handled by mpv (see `crate::player::mpv`) —
//! nothing here touches the audio path a user actually hears. This module
//! only decodes a track (from disk, from downloaded bytes, or parses a
//! source's own precomputed amplitude data) into a small, fixed-length,
//! normalized waveform for display. Every entry point returns `None` on any
//! failure — unsupported codec, corrupt data, I/O error — so callers can
//! fall back to the plain progress bar without special-casing errors.

use std::io::{Cursor, Read, Seek, SeekFrom};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Fixed resolution a waveform is reduced to, independent of terminal
/// width — the UI resamples this further at render time to fit whatever
/// column count it's actually given.
pub const WAVEFORM_BUCKETS: usize = 200;

/// Decode a local file on disk into a bucketed, normalized waveform.
pub fn waveform_from_file(path: &str) -> Option<Vec<f32>> {
    let file = std::fs::File::open(path).ok()?;
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_string);
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    decode_waveform(mss, extension.as_deref())
}

/// Decode an in-memory audio buffer (e.g. a downloaded stream) into a
/// bucketed, normalized waveform. `extension_hint` (e.g. "m4a", "webm")
/// helps symphonia's format probe when the bytes alone are ambiguous.
pub fn waveform_from_bytes(bytes: Vec<u8>, extension_hint: Option<&str>) -> Option<Vec<f32>> {
    let source = ByteSource(Cursor::new(bytes));
    let mss = MediaSourceStream::new(Box::new(source), Default::default());
    decode_waveform(mss, extension_hint)
}

/// Parses a source's own precomputed amplitude data (currently: SoundCloud's
/// `waveform_url` JSON, which stores its samples under either a `samples` or
/// a `data` key depending on which format version served it) into the same
/// bucketed, normalized shape produced by the decode path — callers don't
/// need to know or care which one they got.
pub fn waveform_from_amplitude_json(bytes: &[u8]) -> Option<Vec<f32>> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let raw = value
        .get("samples")
        .or_else(|| value.get("data"))
        .and_then(|v| v.as_array())?;
    let samples: Vec<f32> = raw.iter().filter_map(|v| v.as_f64()).map(|v| v.abs() as f32).collect();
    if samples.is_empty() {
        return None;
    }
    Some(bucket_and_normalize(&samples, WAVEFORM_BUCKETS))
}

/// Wraps an in-memory byte buffer so symphonia can read it like any other
/// seekable source. `std::fs::File` already implements `MediaSource`
/// directly; this small adapter is only needed for bytes that didn't come
/// from disk.
struct ByteSource(Cursor<Vec<u8>>);

impl Read for ByteSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Seek for ByteSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

impl MediaSource for ByteSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.0.get_ref().len() as u64)
    }
}

fn decode_waveform(mss: MediaSourceStream, extension_hint: Option<&str>) -> Option<Vec<f32>> {
    let mut hint = Hint::new();
    if let Some(ext) = extension_hint {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .ok()?;
    let mut format = probed.format;

    let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL)?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .ok()?;

    // Downmixed mono samples, accumulated for the whole track. A few
    // minutes of audio is tens of MB as f32 — a non-issue for a one-shot
    // background task, and far simpler than bucketing on the fly without
    // knowing the true sample count up front.
    let mut mono: Vec<f32> = Vec::new();
    // End of stream, or a stream error mid-track, just stops the loop —
    // whatever was decoded so far is used rather than discarded.
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                buf.copy_interleaved_ref(decoded);
                let channels = spec.channels.count().max(1);
                for frame in buf.samples().chunks_exact(channels) {
                    mono.push(frame.iter().sum::<f32>() / channels as f32);
                }
            }
            // A single bad packet shouldn't sink the whole analysis.
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(_) => break,
        }
    }

    if mono.is_empty() {
        return None;
    }
    Some(bucket_and_normalize(&mono, WAVEFORM_BUCKETS))
}

/// Reduces a sequence of magnitude-ish values (raw downmixed PCM samples, or
/// a source's own already-small amplitude array) to exactly `buckets`
/// values by taking the peak absolute value per chunk, then normalizes the
/// result so its own loudest bucket reaches 1.0 — waveforms read relative to
/// a track's own dynamics, not an absolute scale across tracks.
fn bucket_and_normalize(samples: &[f32], buckets: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; buckets];
    let chunk_len = samples.len().div_ceil(buckets).max(1);
    for (i, chunk) in samples.chunks(chunk_len).enumerate() {
        if i >= buckets {
            break;
        }
        out[i] = chunk.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
    }
    let peak = out.iter().cloned().fold(0.0f32, f32::max);
    if peak > 0.0001 {
        for v in out.iter_mut() {
            *v /= peak;
        }
    }
    out
}
