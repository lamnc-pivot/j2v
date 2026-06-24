//! Real-time audio buffering and transcription worker.
//!
//! Reads `ChunkReceived` streaming events, converts audio to 16 kHz mono,
//! batches short chunks, and forwards them to a persistent Python worker that
//! performs Silero VAD segmentation plus Whisper transcription.

use crate::audio::StreamingEvent;
use crate::transcription::persistent_whisper::PersistentWhisperWorker;
use crossbeam_channel::{Receiver, Sender};
use hound::WavWriter;
use std::io::BufWriter;
use std::path::PathBuf;
use std::thread;

const TARGET_SAMPLE_RATE: u32 = 16_000;
const PUSH_CHUNK_MS: usize = 320;

pub fn spawn_transcription_worker(
    event_rx: Receiver<StreamingEvent>,
    event_tx: Sender<StreamingEvent>,
    capture_sample_rate: u32,
    capture_channels: u16,
    model_dir: PathBuf,
    script_path: PathBuf,
) {
    thread::spawn(move || {
        if let Err(error) = run_worker(
            event_rx,
            event_tx,
            capture_sample_rate,
            capture_channels,
            model_dir,
            script_path,
        ) {
            log::error!("Transcription worker failed: {}", error);
        }
    });
}

fn run_worker(
    event_rx: Receiver<StreamingEvent>,
    event_tx: Sender<StreamingEvent>,
    capture_sample_rate: u32,
    capture_channels: u16,
    model_dir: PathBuf,
    script_path: PathBuf,
) -> Result<(), String> {
    let push_chunk_samples = ((TARGET_SAMPLE_RATE as usize * PUSH_CHUNK_MS) / 1000).max(1);

    let mut worker = PersistentWhisperWorker::new(&model_dir, &script_path)?;
    let mut pending_chunk: Vec<i16> = Vec::with_capacity(push_chunk_samples * 2);
    let mut last_emitted_norm: Option<String> = None;

    log::info!(
        "🧵 Transcription worker started ({}Hz, {}ch, silero-segmented)",
        capture_sample_rate,
        capture_channels
    );

    loop {
        match event_rx.recv() {
            Ok(StreamingEvent::ChunkReceived { chunk }) => {
                let mono = mix_to_mono(&chunk.samples, capture_channels);
                let resampled = if capture_sample_rate == TARGET_SAMPLE_RATE {
                    mono
                } else {
                    downsample(&mono, capture_sample_rate, TARGET_SAMPLE_RATE)
                };

                pending_chunk.extend_from_slice(&resampled);
                if pending_chunk.len() >= push_chunk_samples {
                    let chunk = std::mem::take(&mut pending_chunk);
                    let has_text = transcribe_buffer(
                        &chunk,
                        &mut worker,
                        &event_tx,
                        &mut last_emitted_norm,
                    );
                    if has_text {
                        log::debug!("Transcription worker emitted finalized speech segment(s)");
                    }
                }
            }
            Ok(StreamingEvent::Stopped { .. }) => {
                if !pending_chunk.is_empty() {
                    let chunk = std::mem::take(&mut pending_chunk);
                    let _ = transcribe_buffer(
                        &chunk,
                        &mut worker,
                        &event_tx,
                        &mut last_emitted_norm,
                    );
                }

                let flushed_any = flush_worker_results(
                    &mut worker,
                    &event_tx,
                    &mut last_emitted_norm,
                )?;
                if !flushed_any {
                    log::debug!("No pending speech segments to flush on stop");
                }

                worker.shutdown();
                log::info!("🧵 Transcription worker exiting (stream stopped)");
                return Ok(());
            }
            Ok(_) => {}
            Err(_) => {
                if !pending_chunk.is_empty() {
                    let chunk = std::mem::take(&mut pending_chunk);
                    let _ = transcribe_buffer(
                        &chunk,
                        &mut worker,
                        &event_tx,
                        &mut last_emitted_norm,
                    );
                }

                let _ = flush_worker_results(&mut worker, &event_tx, &mut last_emitted_norm);
                worker.shutdown();
                log::info!("🧵 Transcription worker exiting (channel closed)");
                return Ok(());
            }
        }
    }
}

fn transcribe_buffer(
    pcm: &[i16],
    worker: &mut PersistentWhisperWorker,
    event_tx: &Sender<StreamingEvent>,
    last_emitted_norm: &mut Option<String>,
) -> bool {
    if pcm.is_empty() {
        return false;
    }

    let tmp_path = match write_temp_wav(pcm, TARGET_SAMPLE_RATE) {
        Ok(path) => path,
        Err(e) => {
            log::error!("Failed to write temp WAV: {}", e);
            let _ = event_tx.send(StreamingEvent::TranscriptionError {
                message: format!("WAV write error: {e}"),
            });
            return false;
        }
    };

    match worker.transcribe_file(&tmp_path) {
        Ok(results) => {
            let _ = std::fs::remove_file(&tmp_path);
            emit_transcription_results(results, event_tx, last_emitted_norm)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            log::error!("Transcription error: {}", e);
            let _ = event_tx.send(StreamingEvent::TranscriptionError { message: e });
            false
        }
    }
}

fn flush_worker_results(
    worker: &mut PersistentWhisperWorker,
    event_tx: &Sender<StreamingEvent>,
    last_emitted_norm: &mut Option<String>,
) -> Result<bool, String> {
    let results = worker.flush_pending()?;
    Ok(emit_transcription_results(results, event_tx, last_emitted_norm))
}

fn emit_transcription_results(
    results: Vec<crate::transcription::persistent_whisper::PersistentTranscriptionResult>,
    event_tx: &Sender<StreamingEvent>,
    last_emitted_norm: &mut Option<String>,
) -> bool {
    let mut emitted = false;

    for result in results {
        if result.text.trim().is_empty() {
            continue;
        }

        let norm = normalize_text_for_dedup(&result.text);
        if last_emitted_norm.as_ref() == Some(&norm) {
            log::info!("ℹ️ Suppressed duplicate transcript utterance");
            continue;
        }

        *last_emitted_norm = Some(norm);
        log::info!("📝 Transcribed: {}", result.text);
        let _ = event_tx.send(StreamingEvent::TranscriptionReady {
            text: result.text,
            language: result.language,
            language_probability: result.language_probability,
        });
        emitted = true;
    }

    emitted
}

fn normalize_text_for_dedup(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '。' | '、' | '.' | ',' | '!' | '?' | '！' | '？'))
        .collect()
}

fn mix_to_mono(pcm: &[i16], capture_channels: u16) -> Vec<i16> {
    if capture_channels == 1 {
        return pcm.to_vec();
    }

    let channels = capture_channels as usize;
    pcm.chunks(channels)
        .map(|frame| {
            let sum: i32 = frame.iter().map(|&sample| sample as i32).sum();
            (sum / channels as i32) as i16
        })
        .collect()
}

fn downsample(samples: &[i16], from_rate: u32, to_rate: u32) -> Vec<i16> {
    if from_rate == to_rate {
        return samples.to_vec();
    }

    let ratio = from_rate as f64 / to_rate as f64;
    let out_len = (samples.len() as f64 / ratio).ceil() as usize;
    let mut out = Vec::with_capacity(out_len);
    let window = ratio.ceil() as usize;

    for index in 0..out_len {
        let start = (index as f64 * ratio) as usize;
        let end = (start + window).min(samples.len());
        if start >= samples.len() {
            break;
        }

        let sum: i32 = samples[start..end].iter().map(|&sample| sample as i32).sum();
        let count = (end - start).max(1) as i32;
        out.push((sum / count) as i16);
    }

    out
}

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

    for &sample in samples {
        writer
            .write_sample(sample)
            .map_err(|e| format!("WAV write error: {e}"))?;
    }

    writer
        .finalize()
        .map_err(|e| format!("WAV finalize error: {e}"))?;

    Ok(path)
}