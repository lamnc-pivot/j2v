//! Real-time audio buffering and transcription worker.
//!
//! Reads `ChunkReceived` streaming events, accumulates PCM16 samples,
//! and once a configurable duration threshold is reached:
//!   1. Downsamples from the capture rate (e.g. 48 kHz) to 16 kHz (Whisper requirement)
//!   2. Mixes stereo to mono
//!   3. Writes a temporary WAV file
//!   4. Invokes `faster-whisper` via Python subprocess
//!   5. Emits a `TranscriptionReady` event back on the same channel

use crate::audio::StreamingEvent;
use crate::transcription;
use crossbeam_channel::{Receiver, Sender};
use hound::WavWriter;
use std::io::BufWriter;
use std::path::PathBuf;
use std::thread;

/// Seconds of audio to buffer before triggering a transcription pass.
const BUFFER_DURATION_SECS: f32 = 3.0;
/// Extended window used when signal is weak to collect more speech context.
const LOW_SIGNAL_BUFFER_DURATION_SECS: f32 = 5.0;
/// RMS below this value is treated as weak-signal window for adaptive sizing.
const LOW_SIGNAL_WINDOW_DBFS: f32 = -58.0;

/// Seconds of overlap kept from the previous buffer for continuity.
const OVERLAP_SECS: f32 = 0.5;

/// Hard skip threshold for near-silence windows.
/// Values lower than this are usually silence/noise floor.
const HARD_MIN_RMS_DBFS: f32 = -75.0;
/// Soft warning threshold; we still decode below this to avoid missing quiet speech.
const SOFT_WARN_RMS_DBFS: f32 = -60.0;
/// For very weak windows, require stricter acceptance for transcription text.
const LOW_SIGNAL_ACCEPT_DBFS: f32 = -60.0;
/// Minimum confidence to accept low-signal transcripts.
const LOW_SIGNAL_MIN_LANG_PROB: f32 = 0.92;
/// Target RMS level for whisper input after normalization.
const AUTO_GAIN_TARGET_DBFS: f32 = -28.0;
/// Max gain boost to avoid over-amplifying noise too aggressively.
const AUTO_GAIN_MAX_DB: f32 = 44.0;
/// Minimum voiced-frame ratio required before invoking Whisper.
const VAD_PRE_GATE_MIN_RATIO: f32 = 0.03;
/// Frame size for lightweight VAD pre-gate.
const VAD_FRAME_MS: usize = 20;

/// Spawns a background worker thread that:
///   - Reads events from `event_rx`
///   - Accumulates audio into a rolling buffer
///   - Triggers Whisper transcription every `BUFFER_DURATION_SECS`
///   - Sends `TranscriptionReady` (or `Error`) events back on `event_tx`
///
/// The worker exits automatically when either channel closes or a `Stopped` event arrives.
pub fn spawn_transcription_worker(
    event_rx: Receiver<StreamingEvent>,
    event_tx: Sender<StreamingEvent>,
    capture_sample_rate: u32,
    capture_channels: u16,
    model_dir: PathBuf,
    script_path: PathBuf,
) {
    thread::spawn(move || {
        run_worker(
            event_rx,
            event_tx,
            capture_sample_rate,
            capture_channels,
            model_dir,
            script_path,
        );
    });
}

fn run_worker(
    event_rx: Receiver<StreamingEvent>,
    event_tx: Sender<StreamingEvent>,
    capture_sample_rate: u32,
    capture_channels: u16,
    model_dir: PathBuf,
    script_path: PathBuf,
) {
    let base_buffer_samples = (capture_sample_rate as f32 * BUFFER_DURATION_SECS) as usize
        * capture_channels as usize;
    let low_signal_buffer_samples = (capture_sample_rate as f32 * LOW_SIGNAL_BUFFER_DURATION_SECS) as usize
        * capture_channels as usize;
    let overlap_samples = (capture_sample_rate as f32 * OVERLAP_SECS) as usize
        * capture_channels as usize;

    let mut pcm_buffer: Vec<i16> = Vec::with_capacity(base_buffer_samples * 2);
    let mut last_emitted_norm: Option<String> = None;
    let mut empty_streak: usize = 0;

    log::info!(
        "🧵 Transcription worker started ({}Hz, {}ch, buffer={:.1}s)",
        capture_sample_rate,
        capture_channels,
        BUFFER_DURATION_SECS
    );

    loop {
        match event_rx.recv() {
            Ok(StreamingEvent::ChunkReceived { chunk }) => {
                pcm_buffer.extend_from_slice(&chunk.samples);

                if pcm_buffer.len() >= base_buffer_samples {
                    let recent_start = pcm_buffer.len().saturating_sub(base_buffer_samples);
                    let recent = &pcm_buffer[recent_start..];
                    let recent_rms_dbfs = calculate_rms_dbfs(recent);
                    let weak_signal = recent_rms_dbfs < LOW_SIGNAL_WINDOW_DBFS;
                    let chosen_window_samples = if weak_signal {
                        low_signal_buffer_samples
                    } else {
                        base_buffer_samples
                    };

                    if weak_signal && pcm_buffer.len() < chosen_window_samples {
                        // Collect more context (up to 5s) for weak-signal windows.
                        continue;
                    }

                    // Take a buffer-sized slice and keep overlap for next pass
                    let window_samples = chosen_window_samples.min(pcm_buffer.len());
                    let slice = pcm_buffer[..window_samples].to_vec();
                    let keep = if pcm_buffer.len() > overlap_samples {
                        pcm_buffer[pcm_buffer.len() - overlap_samples..].to_vec()
                    } else {
                        pcm_buffer.clone()
                    };
                    pcm_buffer = keep;

                    let has_text = transcribe_buffer(
                        &slice,
                        capture_sample_rate,
                        capture_channels,
                        &model_dir,
                        &script_path,
                        &event_tx,
                        &mut last_emitted_norm,
                    );
                    update_empty_streak(has_text, &mut empty_streak);
                }
            }

            Ok(StreamingEvent::Stopped { .. }) => {
                // Transcribe any remaining audio on stop
                if !pcm_buffer.is_empty() {
                    let has_text = transcribe_buffer(
                        &pcm_buffer,
                        capture_sample_rate,
                        capture_channels,
                        &model_dir,
                        &script_path,
                        &event_tx,
                        &mut last_emitted_norm,
                    );
                    update_empty_streak(has_text, &mut empty_streak);
                }
                log::info!("🧵 Transcription worker exiting (stream stopped)");
                break;
            }

            Ok(_) => {} // Ignore Started / Error events from audio capture

            Err(_) => {
                log::info!("🧵 Transcription worker exiting (channel closed)");
                break;
            }
        }
    }
}

/// Downsamples, mono-mixes, writes temp WAV, and dispatches the Whisper subprocess.
fn transcribe_buffer(
    pcm: &[i16],
    capture_sample_rate: u32,
    capture_channels: u16,
    model_dir: &PathBuf,
    script_path: &PathBuf,
    event_tx: &Sender<StreamingEvent>,
    last_emitted_norm: &mut Option<String>,
) -> bool {
    // ── 1. Mix channels → mono ──────────────────────────────────────────────
    let mono: Vec<i16> = if capture_channels == 1 {
        pcm.to_vec()
    } else {
        let ch = capture_channels as usize;
        pcm.chunks(ch)
            .map(|frame| {
                let sum: i32 = frame.iter().map(|&s| s as i32).sum();
                (sum / ch as i32) as i16
            })
            .collect()
    };

    // ── 2. Downsample to 16 kHz ─────────────────────────────────────────────
    let target_rate: u32 = 16_000;
    let resampled = if capture_sample_rate == target_rate {
        mono
    } else {
        downsample(&mono, capture_sample_rate, target_rate)
    };

    if resampled.is_empty() {
        log::debug!("Skipping transcription: empty buffer after resampling");
        return false;
    }

    let rms_dbfs = calculate_rms_dbfs(&resampled);
    if rms_dbfs < HARD_MIN_RMS_DBFS {
        log::debug!(
            "Skip very-low-energy window ({:.1} dBFS < {:.1} dBFS)",
            rms_dbfs,
            HARD_MIN_RMS_DBFS
        );
        let _ = event_tx.send(StreamingEvent::TranscriptionReady {
            text: String::new(),
            language: "ja".to_string(),
            language_probability: 0.0,
        });
        return false;
    }

    if rms_dbfs < SOFT_WARN_RMS_DBFS {
        log::debug!("Low-energy window ({:.1} dBFS) - still decoding", rms_dbfs);
    }

    // Normalize low-level input from headsets/loopback before whisper decode.
    let (normalized, gain_db, normalized_rms_dbfs) =
        apply_auto_gain(&resampled, AUTO_GAIN_TARGET_DBFS, AUTO_GAIN_MAX_DB);
    if gain_db > 0.1 {
        if gain_db >= 6.0 {
            log::info!(
                "ℹ️ Applied auto-gain +{:.1} dB (RMS {:.1} -> {:.1} dBFS)",
                gain_db,
                rms_dbfs,
                normalized_rms_dbfs
            );
        } else {
            log::debug!(
                "Applied auto-gain +{:.1} dB (RMS {:.1} -> {:.1} dBFS)",
                gain_db,
                rms_dbfs,
                normalized_rms_dbfs
            );
        }
    }

    let vad_ratio = estimate_voice_activity_ratio(&normalized, target_rate, VAD_FRAME_MS);
    // If RMS is already strong, don't let pre-gate block decode.
    if vad_ratio < VAD_PRE_GATE_MIN_RATIO && normalized_rms_dbfs < -36.0 {
        log::debug!(
            "VAD pre-gate skipped decode (ratio {:.3} < {:.3})",
            vad_ratio,
            VAD_PRE_GATE_MIN_RATIO
        );
        let _ = event_tx.send(StreamingEvent::TranscriptionReady {
            text: String::new(),
            language: "ja".to_string(),
            language_probability: 0.0,
        });
        return false;
    }

    // ── 3. Write temp WAV ───────────────────────────────────────────────────
    let tmp_path = match write_temp_wav(&normalized, target_rate) {
        Ok(p) => p,
        Err(e) => {
            log::error!("Failed to write temp WAV: {}", e);
            let _ = event_tx.send(StreamingEvent::TranscriptionError {
                message: format!("WAV write error: {e}"),
            });
            return false;
        }
    };

    log::debug!(
        "Transcribing {} samples ({:.2}s) from {}",
        normalized.len(),
        normalized.len() as f32 / target_rate as f32,
        tmp_path.display()
    );

    // ── 4. Call Whisper ─────────────────────────────────────────────────────
    match transcription::transcribe_wav(&tmp_path, model_dir, script_path) {
        Ok(result) => {
            let _ = std::fs::remove_file(&tmp_path);
            if !result.text.trim().is_empty() {
                if should_reject_low_signal_result(
                    &result.text,
                    &result.language,
                    result.language_probability,
                    normalized_rms_dbfs,
                ) {
                    log::info!(
                        "ℹ️ Rejected low-signal transcript (lang={}, p={:.2}, {:.1} dBFS)",
                        result.language,
                        result.language_probability,
                        normalized_rms_dbfs
                    );
                    let _ = event_tx.send(StreamingEvent::TranscriptionReady {
                        text: String::new(),
                        language: result.language,
                        language_probability: result.language_probability,
                    });
                    return false;
                }

                let norm = normalize_text_for_dedup(&result.text);
                if last_emitted_norm.as_ref() == Some(&norm) {
                    if normalized_rms_dbfs < SOFT_WARN_RMS_DBFS {
                        log::debug!(
                            "Suppressed duplicate transcript on low-energy window ({:.1} dBFS)",
                            normalized_rms_dbfs
                        );
                    } else {
                        log::info!("ℹ️ Suppressed duplicate transcript window");
                    }
                    let _ = event_tx.send(StreamingEvent::TranscriptionReady {
                        text: String::new(),
                        language: result.language,
                        language_probability: result.language_probability,
                    });
                    return false;
                }

                *last_emitted_norm = Some(norm);
                log::info!("📝 Transcribed: {}", result.text);
                let _ = event_tx.send(StreamingEvent::TranscriptionReady {
                    text: result.text,
                    language: result.language,
                    language_probability: result.language_probability,
                });
                return true;
            } else {
                log::debug!("Transcription processed window but produced empty text");
            }
            let _ = event_tx.send(StreamingEvent::TranscriptionReady {
                text: result.text,
                language: result.language,
                language_probability: result.language_probability,
            });
            false
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            log::error!("Transcription error: {}", e);
            let _ = event_tx.send(StreamingEvent::TranscriptionError { message: e });
            false
        }
    }
}

fn update_empty_streak(has_text: bool, empty_streak: &mut usize) {
    if has_text {
        if *empty_streak >= 3 {
            log::info!("ℹ️ Empty windows streak ended after {} window(s)", *empty_streak);
        }
        *empty_streak = 0;
        return;
    }

    *empty_streak += 1;
    if *empty_streak % 5 == 0 {
        log::info!(
            "ℹ️ No transcript in last {} window(s) (likely silence/noise)",
            *empty_streak
        );
    }
}

fn calculate_rms_dbfs(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return -100.0;
    }

    let sum_sq: f64 = samples
        .iter()
        .map(|&s| {
            let x = s as f64 / i16::MAX as f64;
            x * x
        })
        .sum();
    let rms = (sum_sq / samples.len() as f64).sqrt();
    if rms <= f64::EPSILON {
        return -100.0;
    }

    (20.0 * rms.log10()) as f32
}

fn apply_auto_gain(samples: &[i16], target_dbfs: f32, max_gain_db: f32) -> (Vec<i16>, f32, f32) {
    if samples.is_empty() {
        return (Vec::new(), 0.0, -100.0);
    }

    let before_dbfs = calculate_rms_dbfs(samples);
    if before_dbfs <= -99.0 {
        return (samples.to_vec(), 0.0, before_dbfs);
    }

    let desired_gain_db = (target_dbfs - before_dbfs).max(0.0).min(max_gain_db);
    if desired_gain_db <= 0.01 {
        return (samples.to_vec(), 0.0, before_dbfs);
    }

    let peak_norm = samples
        .iter()
        .map(|s| (s.abs() as f32) / (i16::MAX as f32))
        .fold(0.0_f32, f32::max);

    // Keep ~1dB headroom to avoid clipping after amplification.
    let max_peak_norm = 0.89_f32;
    let peak_limited_gain_db = if peak_norm > 0.0 {
        (20.0 * (max_peak_norm / peak_norm).log10()).max(0.0)
    } else {
        desired_gain_db
    };

    let gain_db = desired_gain_db.min(peak_limited_gain_db);
    if gain_db <= 0.01 {
        return (samples.to_vec(), 0.0, before_dbfs);
    }

    let gain = 10f32.powf(gain_db / 20.0);
    let amplified: Vec<i16> = samples
        .iter()
        .map(|&s| {
            let y = (s as f32) * gain;
            y.clamp(i16::MIN as f32, i16::MAX as f32) as i16
        })
        .collect();

    let after_dbfs = calculate_rms_dbfs(&amplified);
    (amplified, gain_db, after_dbfs)
}

fn normalize_text_for_dedup(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '。' | '、' | '.' | ',' | '!' | '?' | '！' | '？'))
        .collect()
}

fn estimate_voice_activity_ratio(samples: &[i16], sample_rate: u32, frame_ms: usize) -> f32 {
    if samples.is_empty() || sample_rate == 0 {
        return 0.0;
    }

    let frame_size = ((sample_rate as usize * frame_ms) / 1000).max(1);
    let mut voiced = 0usize;
    let mut total = 0usize;

    for frame in samples.chunks(frame_size) {
        if frame.is_empty() {
            continue;
        }
        total += 1;
        let dbfs = calculate_rms_dbfs(frame);
        // Headset and loopback sources can be much lower than close-talk mics.
        if dbfs > -50.0 {
            voiced += 1;
        }
    }

    if total == 0 {
        0.0
    } else {
        voiced as f32 / total as f32
    }
}

fn should_reject_low_signal_result(
    text: &str,
    language: &str,
    language_probability: f32,
    rms_dbfs: f32,
) -> bool {
    if rms_dbfs >= LOW_SIGNAL_ACCEPT_DBFS {
        return false;
    }

    // In weak signal zones, only trust high-confidence Japanese output.
    if language != "ja" {
        return true;
    }

    if language_probability < LOW_SIGNAL_MIN_LANG_PROB {
        return true;
    }

    // Guard against tiny filler outputs in noise-only windows.
    let normalized = normalize_text_for_dedup(text);
    normalized.chars().count() <= 1
}

/// Simple linear-interpolation downsampler for mono i16 audio.
///
/// For integer ratios (e.g. 48000→16000, ratio=3) this averages groups of
/// `ratio` samples (box filter), which removes aliasing at the cost of a
/// gentle roll-off above the new Nyquist frequency.
fn downsample(samples: &[i16], from_rate: u32, to_rate: u32) -> Vec<i16> {
    if from_rate == to_rate {
        return samples.to_vec();
    }

    let ratio = from_rate as f64 / to_rate as f64;
    let out_len = (samples.len() as f64 / ratio).ceil() as usize;
    let mut out = Vec::with_capacity(out_len);

    let window = ratio.ceil() as usize;

    for i in 0..out_len {
        let start = (i as f64 * ratio) as usize;
        let end = (start + window).min(samples.len());
        if start >= samples.len() {
            break;
        }
        let sum: i32 = samples[start..end].iter().map(|&s| s as i32).sum();
        let count = (end - start).max(1) as i32;
        out.push((sum / count) as i16);
    }

    out
}

/// Writes mono PCM16 samples to a temporary WAV file at `sample_rate`.
fn write_temp_wav(samples: &[i16], sample_rate: u32) -> Result<PathBuf, String> {
    let tmp_dir = std::env::temp_dir();
    let filename = format!(
        "j2v_whisper_{}.wav",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );
    let path = tmp_dir.join(filename);

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let file = std::fs::File::create(&path)
        .map_err(|e| format!("Cannot create temp file: {e}"))?;
    let mut writer = WavWriter::new(BufWriter::new(file), spec)
        .map_err(|e| format!("Cannot create WAV writer: {e}"))?;

    for &s in samples {
        writer
            .write_sample(s)
            .map_err(|e| format!("WAV write error: {e}"))?;
    }
    writer
        .finalize()
        .map_err(|e| format!("WAV finalize error: {e}"))?;

    Ok(path)
}
