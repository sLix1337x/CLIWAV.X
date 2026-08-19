//! WASAPI loopback capture of system audio output, for the live terminal
//! visualizer.
//!
//! Playback goes through mpv as a separate process controlled over JSON
//! IPC — we never see mpv's raw PCM, only playback-control properties. mpv
//! also doesn't expose a clean "give me the current spectrum" property, and
//! empirically its `af`/filter-command surface isn't reliable enough to lean
//! on for that either — `af-command` returned "error running command" for
//! every runtime-commandable filter tested against a live mpv v0.41, so the
//! EQ drives full-chain `af set` replacements instead. Capturing the system's
//! audio *output* independently — via WASAPI's loopback mode, which mirrors
//! whatever a render device is currently playing — sidesteps mpv entirely:
//! it captures whatever is actually coming out of the speakers, regardless
//! of which source is playing.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use wasapi::{deinitialize, initialize_mta, Direction, DeviceEnumerator, SampleType, StreamMode, WaveFormat};

/// Loopback-captured audio is downmixed to mono and delivered at this fixed
/// rate — requested from WASAPI with `autoconvert: true`, so the audio
/// engine resamples for us regardless of the output device's native format.
pub const SAMPLE_RATE: u32 = 48_000;

/// Ring buffer capacity in mono samples — a little under 3 seconds at
/// `SAMPLE_RATE`, comfortably more than the largest single read the
/// spectrum analyzer does.
const RING_CAPACITY: usize = 131_072;

/// One stereo f32 frame: 2 channels * 4 bytes.
const BYTES_PER_FRAME: usize = 8;

pub struct AudioCapture {
    ring: Arc<Mutex<VecDeque<f32>>>,
    running: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl AudioCapture {
    /// Spawns the capture thread and blocks briefly for it to confirm WASAPI
    /// initialized successfully. Returns `None` on any failure (no default
    /// output device, format negotiation refused, ...) — the caller treats
    /// that as "no visualizer this session" rather than a fatal error.
    pub fn start() -> Option<Self> {
        let ring = Arc::new(Mutex::new(VecDeque::with_capacity(RING_CAPACITY)));
        let running = Arc::new(AtomicBool::new(true));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<bool>();

        let thread_ring = Arc::clone(&ring);
        let thread_running = Arc::clone(&running);
        let handle = thread::Builder::new()
            .name("wasapi-loopback".to_string())
            .spawn(move || run(&thread_ring, &thread_running, &ready_tx))
            .ok()?;

        match ready_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(true) => Some(Self {
                ring,
                running,
                thread: Some(handle),
            }),
            _ => {
                running.store(false, Ordering::Relaxed);
                let _ = handle.join();
                None
            }
        }
    }

    /// Copies out the most recent `n` mono samples, oldest first. Shorter
    /// than `n` if the capture thread hasn't produced that much yet (e.g.
    /// right after starting).
    pub fn latest(&self, n: usize) -> Vec<f32> {
        let ring = self.ring.lock().unwrap();
        let skip = ring.len().saturating_sub(n);
        ring.iter().skip(skip).copied().collect()
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

fn run(ring: &Arc<Mutex<VecDeque<f32>>>, running: &Arc<AtomicBool>, ready_tx: &Sender<bool>) {
    if let Err(e) = capture(ring, running, ready_tx) {
        tracing::warn!("visualizer: WASAPI loopback capture stopped: {e}");
    }
}

fn capture(
    ring: &Arc<Mutex<VecDeque<f32>>>,
    running: &Arc<AtomicBool>,
    ready_tx: &Sender<bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    initialize_mta().ok()?;
    let result = capture_inner(ring, running, ready_tx);
    deinitialize();
    result
}

fn capture_inner(
    ring: &Arc<Mutex<VecDeque<f32>>>,
    running: &Arc<AtomicBool>,
    ready_tx: &Sender<bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let enumerator = DeviceEnumerator::new()?;
    // Loopback capture: get the default RENDER (output) device, but
    // initialize its client for the CAPTURE direction below — wasapi-rs
    // turns that specific (Render device, Capture direction, Shared mode)
    // combination into the underlying AUDCLNT_STREAMFLAGS_LOOPBACK flag,
    // which makes the client mirror whatever the device is playing instead
    // of trying to record a microphone.
    let device = enumerator.get_default_device(&Direction::Render)?;
    let mut audio_client = device.get_iaudioclient()?;
    let desired_format = WaveFormat::new(32, 32, &SampleType::Float, SAMPLE_RATE as usize, 2, None);

    let (_, min_time) = audio_client.get_device_period()?;
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: min_time,
    };
    audio_client.initialize_client(&desired_format, &Direction::Capture, &mode)?;

    let h_event = audio_client.set_get_eventhandle()?;
    let capture_client = audio_client.get_audiocaptureclient()?;
    audio_client.start_stream()?;
    let _ = ready_tx.send(true);

    let mut byte_queue: VecDeque<u8> = VecDeque::with_capacity(64 * 1024);

    while running.load(Ordering::Relaxed) {
        // A 200ms wait timeout doubles as the stop-flag poll interval —
        // `EventTimeout` isn't a real failure here, just a chance to check
        // whether we've been asked to stop.
        if h_event.wait_for_event(200).is_err() {
            continue;
        }
        capture_client.read_from_device_to_deque(&mut byte_queue)?;

        let mut mono = Vec::with_capacity(byte_queue.len() / BYTES_PER_FRAME);
        while byte_queue.len() >= BYTES_PER_FRAME {
            let l = read_f32(&mut byte_queue);
            let r = read_f32(&mut byte_queue);
            mono.push((l + r) * 0.5);
        }
        if mono.is_empty() {
            continue;
        }

        let mut buf = ring.lock().unwrap();
        buf.extend(mono);
        let excess = buf.len().saturating_sub(RING_CAPACITY);
        if excess > 0 {
            buf.drain(..excess);
        }
    }

    let _ = audio_client.stop_stream();
    Ok(())
}

fn read_f32(queue: &mut VecDeque<u8>) -> f32 {
    let bytes = [
        queue.pop_front().unwrap_or(0),
        queue.pop_front().unwrap_or(0),
        queue.pop_front().unwrap_or(0),
        queue.pop_front().unwrap_or(0),
    ];
    f32::from_le_bytes(bytes)
}
