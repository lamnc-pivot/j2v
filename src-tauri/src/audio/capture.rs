//! Real-time audio capture with multi-format support.
//!
//! Captures audio from the default input device and writes to a WAV file.
//! Supports automatic conversion from I16, U16, and F32 sample formats to PCM16.
//! Supports streaming audio chunks in real-time via event channel.

use crate::audio::{AudioCaptureConfig, AudioChunk, CaptureBackend, StreamingEvent};
use chrono::Local;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Stream, StreamConfig};
use hound::WavWriter;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(target_os = "macos")]
use screencapturekit::cm::AudioBufferList;
#[cfg(target_os = "macos")]
use screencapturekit::prelude::*;
#[cfg(target_os = "windows")]
use std::collections::VecDeque;
#[cfg(target_os = "windows")]
use std::sync::mpsc;
#[cfg(target_os = "windows")]
use std::thread::JoinHandle;
#[cfg(target_os = "windows")]
use wasapi::{initialize_mta, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

/// Metadata about captured audio
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AudioData {
    /// Duration of the recording in seconds
    pub duration_secs: f32,
    /// Total number of samples captured
    pub sample_count: usize,
    /// Sample rate in Hz (e.g., 48000)
    pub sample_rate: u32,
    /// Number of audio channels
    pub channels: u16,
    /// Path to saved WAV file
    pub file_path: Option<String>,
}

/// Manages audio capture session lifecycle and state.
pub struct AudioCapture {
    config: AudioCaptureConfig,
    stream: Option<Stream>,
    is_recording: Arc<AtomicBool>,
    start_time: Option<Instant>,
    samples_collected: Arc<Mutex<usize>>,
    writer: Option<Arc<Mutex<Option<WavWriter<std::io::BufWriter<File>>>>>>,
    temp_file_path: Option<PathBuf>,
    /// Channel for sending audio chunks (streaming mode)
    event_tx: Option<crossbeam_channel::Sender<StreamingEvent>>,
    /// Timestamp of start for chunk timestamping (milliseconds since UNIX_EPOCH)
    start_timestamp_ms: Arc<AtomicU64>,
    #[cfg(target_os = "macos")]
    sck_stream: Option<SCStream>,
    #[cfg(target_os = "windows")]
    wasapi_stop_tx: Option<mpsc::Sender<()>>,
    #[cfg(target_os = "windows")]
    wasapi_thread: Option<JoinHandle<()>>,
}

impl AudioCapture {
    /// Creates a new audio capture instance with the given configuration.
    pub fn new(config: AudioCaptureConfig) -> Self {
        log::info!(
            "📦 Creating AudioCapture: {}Hz, {}ch",
            config.sample_rate,
            config.channels
        );

        Self {
            config,
            stream: None,
            is_recording: Arc::new(AtomicBool::new(false)),
            start_time: None,
            samples_collected: Arc::new(Mutex::new(0)),
            writer: None,
            temp_file_path: None,
            event_tx: None,
            start_timestamp_ms: Arc::new(AtomicU64::new(0)),
            #[cfg(target_os = "macos")]
            sck_stream: None,
            #[cfg(target_os = "windows")]
            wasapi_stop_tx: None,
            #[cfg(target_os = "windows")]
            wasapi_thread: None,
        }
    }

    /// Sets up the event channel for streaming audio chunks.
    pub fn set_event_channel(&mut self, tx: crossbeam_channel::Sender<StreamingEvent>) {
        self.event_tx = Some(tx);
    }

    /// Starts capturing audio from the default input device.
    ///
    /// # Errors
    /// Returns an error if:
    /// - No input device is available
    /// - Failed to create output file
    /// - Device configuration is not supported
    /// - Recording is already in progress
    pub fn start(&mut self) -> Result<(), String> {
        if self.is_recording.load(Ordering::SeqCst) {
            return Err("Already recording".to_string());
        }

        log::info!("🎙️ Starting audio capture");

        #[cfg(target_os = "macos")]
        if matches!(
            self.config.capture_backend,
            CaptureBackend::ScreenCaptureKit | CaptureBackend::Auto
        ) {
            return self.start_screen_capture_kit();
        }

        #[cfg(target_os = "windows")]
        if matches!(
            self.config.capture_backend,
            CaptureBackend::WasapiLoopback | CaptureBackend::Auto
        ) {
            return self.start_wasapi_loopback();
        }

        let device = self.get_input_device()?;
        let config = self.setup_device_config(&device)?;
        // Use the real device format so downstream processing stays aligned.
        self.config.sample_rate = config.sample_rate().0;
        self.config.channels = config.channels();
        let temp_file_path = self.create_output_file()?;

        let sample_format = config.sample_format();
        let stream_config: StreamConfig = config.clone().into();

        // Initialize recording state
        self.is_recording.store(true, Ordering::SeqCst);
        self.start_time = Some(Instant::now());
        
        // Record start timestamp for chunk timestamping
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.start_timestamp_ms.store(now, Ordering::SeqCst);
        
        *self.samples_collected.lock() = 0;
        self.temp_file_path = Some(temp_file_path);

        // Emit Started event
        if let Some(ref tx) = self.event_tx {
            let _ = tx.try_send(StreamingEvent::Started {
                sample_rate: self.config.sample_rate,
                channels: self.config.channels,
            });
        }

        // Create WAV writer
        let writer = self.create_wav_writer(self.temp_file_path.as_ref().unwrap())?;
        let writer = Arc::new(Mutex::new(Some(writer)));

        // Build appropriate stream for detected sample format
        let is_recording = Arc::clone(&self.is_recording);
        let samples_collected = Arc::clone(&self.samples_collected);
        let writer_clone = Arc::clone(&writer);
        let event_tx = self.event_tx.clone();
        let start_timestamp_ms = Arc::clone(&self.start_timestamp_ms);

        let stream = self.build_stream(
            &device,
            &stream_config,
            sample_format,
            is_recording,
            samples_collected,
            writer_clone,
            event_tx,
            start_timestamp_ms,
        )?;
        stream
            .play()
            .map_err(|e| format!("Failed to start stream: {}", e))?;

        self.stream = Some(stream);
        self.writer = Some(writer);

        log::info!("✅ Audio capture started");
        Ok(())
    }

    /// Stops capturing audio and returns metadata about the recording.
    pub fn stop(&mut self) -> Result<AudioData, String> {
        if !self.is_recording.load(Ordering::SeqCst) {
            return Err("Not currently recording".to_string());
        }

        log::info!("⏹️ Stopping audio capture");
        self.is_recording.store(false, Ordering::SeqCst);

        #[cfg(target_os = "macos")]
        if let Some(sck_stream) = self.sck_stream.take() {
            sck_stream
                .stop_capture()
                .map_err(|e| format!("Failed to stop ScreenCaptureKit stream: {e}"))?;
        }

        #[cfg(target_os = "windows")]
        if let Some(tx) = self.wasapi_stop_tx.take() {
            let _ = tx.send(());
        }
        #[cfg(target_os = "windows")]
        if let Some(handle) = self.wasapi_thread.take() {
            let _ = handle.join();
        }

        // Flush audio data
        drop(self.stream.take());
        std::thread::sleep(Duration::from_millis(100));

        // Finalize WAV file
        if let Some(writer_arc) = self.writer.take() {
            if let Some(writer) = writer_arc.lock().take() {
                writer
                    .finalize()
                    .map_err(|e| format!("Failed to finalize WAV: {}", e))?;
            }
        }

        let duration = self
            .start_time
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0);

        let samples = *self.samples_collected.lock();
        let file_path = self
            .temp_file_path
            .take()
            .map(|p| p.to_string_lossy().to_string());

        log::info!(
            "✅ Audio capture stopped: {:.2}s, {} samples",
            duration,
            samples
        );

        // Emit Stopped event
        if let Some(ref tx) = self.event_tx {
            let _ = tx.try_send(StreamingEvent::Stopped {
                duration_secs: duration,
                sample_count: samples,
                file_path: file_path.clone(),
            });
        }

        Ok(AudioData {
            duration_secs: duration,
            sample_count: samples,
            sample_rate: self.config.sample_rate,
            channels: self.config.channels,
            file_path,
        })
    }

    /// Returns whether recording is currently active.
    pub fn is_recording(&self) -> bool {
        self.is_recording.load(Ordering::SeqCst)
    }

    /// Returns the current recording duration in seconds.
    pub fn get_duration(&self) -> f32 {
        self.start_time
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0)
    }

    /// Returns the active capture sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate
    }

    /// Returns the active capture channel count.
    pub fn channels(&self) -> u16 {
        self.config.channels
    }

    // Private helper methods

    /// Gets a loopback-backed input device for output-audio capture.
    fn get_input_device(&self) -> Result<Device, String> {
        match self.config.capture_backend {
            CaptureBackend::Auto | CaptureBackend::LoopbackDriver => {
                self.get_loopback_input_device()
            }
            CaptureBackend::ScreenCaptureKit => {
                #[cfg(target_os = "macos")]
                {
                    return Err("ScreenCaptureKit path does not use cpal input devices".to_string());
                }
                #[cfg(not(target_os = "macos"))]
                {
                    return Err("ScreenCaptureKit chỉ hỗ trợ macOS".to_string());
                }
            }
            CaptureBackend::WasapiLoopback => {
                #[cfg(target_os = "windows")]
                {
                    return Err(
                        "WASAPI loopback backend chưa được triển khai ở bản này. Hãy chọn Loopback Driver tạm thời."
                            .to_string(),
                    );
                }
                #[cfg(not(target_os = "windows"))]
                {
                    return Err("WASAPI loopback chỉ hỗ trợ Windows".to_string());
                }
            }
        }
    }

    fn get_loopback_input_device(&self) -> Result<Device, String> {
        let host = cpal::default_host();
        let devices = host
            .input_devices()
            .map_err(|e| format!("Failed to list input devices: {}", e))?;

        let mut best: Option<(i32, String, Device)> = None;

        for device in devices {
            let name = device.name().unwrap_or_else(|_| "Unknown".to_string());
            let score = score_loopback_device_name(&name);
            if score <= 0 {
                continue;
            }

            if let Some((best_score, _, _)) = &best {
                if score > *best_score {
                    best = Some((score, name, device));
                }
            } else {
                best = Some((score, name, device));
            }
        }

        if let Some((score, name, device)) = best {
            log::info!("  Using loopback device: {} (score={})", name, score);
            return Ok(device);
        }

        log::error!("❌ No loopback/system-audio input device found");
        Err(
            "No loopback/system-audio device found. Install and route audio via BlackHole (macOS) or Stereo Mix/VB-Cable (Windows)."
                .to_string(),
        )
    }

    /// Configures the device and returns the configuration
    fn setup_device_config(&self, device: &Device) -> Result<cpal::SupportedStreamConfig, String> {
        let name = device.name().unwrap_or_else(|_| "Unknown".to_string());
        log::info!("  Device: {}", name);

        let config = device
            .default_input_config()
            .map_err(|e| format!("Failed to get device config: {}", e))?;

        log::info!(
            "  Format: {}Hz, {} channels",
            config.sample_rate().0,
            config.channels()
        );

        Ok(config)
    }

    /// Creates the output file and returns the path
    fn create_output_file(&self) -> Result<PathBuf, String> {
        let cache_dir = dirs::cache_dir().ok_or_else(|| "Cache directory not found".to_string())?;

        if !cache_dir.exists() {
            fs::create_dir_all(&cache_dir)
                .map_err(|e| format!("Failed to create cache dir: {}", e))?;
        }

        let file_path = cache_dir.join(format!(
            "j2v_audio_capture_{}.wav",
            Local::now().format("%Y%m%d_%H%M%S")
        ));

        log::info!("  Output: {}", file_path.display());
        Ok(file_path)
    }

    /// Creates a WAV writer for the output file
    fn create_wav_writer(
        &self,
        path: &PathBuf,
    ) -> Result<WavWriter<std::io::BufWriter<File>>, String> {
        let file = File::create(path).map_err(|e| format!("Failed to create file: {}", e))?;

        let spec = hound::WavSpec {
            sample_format: hound::SampleFormat::Int,
            channels: self.config.channels,
            sample_rate: self.config.sample_rate,
            bits_per_sample: 16,
        };

        WavWriter::new(std::io::BufWriter::new(file), spec)
            .map_err(|e| format!("Failed to create WAV writer: {}", e))
    }

    #[cfg(target_os = "macos")]
    fn start_screen_capture_kit(&mut self) -> Result<(), String> {
        let content = SCShareableContent::get()
            .map_err(|e| format!("Failed to query shareable content: {e}"))?;
        let display = content
            .displays()
            .into_iter()
            .next()
            .ok_or_else(|| "No display available for ScreenCaptureKit".to_string())?;

        let filter = SCContentFilter::create()
            .with_display(&display)
            .with_excluding_windows(&[])
            .build();

        let config = SCStreamConfiguration::new()
            .with_captures_audio(true)
            .with_sample_rate(self.config.sample_rate as i32)
            .with_channel_count(self.config.channels as i32);

        let temp_file_path = self.create_output_file()?;

        self.is_recording.store(true, Ordering::SeqCst);
        self.start_time = Some(Instant::now());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.start_timestamp_ms.store(now, Ordering::SeqCst);
        *self.samples_collected.lock() = 0;
        self.temp_file_path = Some(temp_file_path);

        if let Some(ref tx) = self.event_tx {
            let _ = tx.try_send(StreamingEvent::Started {
                sample_rate: self.config.sample_rate,
                channels: self.config.channels,
            });
        }

        let writer = self.create_wav_writer(self.temp_file_path.as_ref().unwrap())?;
        let writer = Arc::new(Mutex::new(Some(writer)));

        let is_recording = Arc::clone(&self.is_recording);
        let samples_collected = Arc::clone(&self.samples_collected);
        let writer_clone = Arc::clone(&writer);
        let event_tx = self.event_tx.clone();
        let start_timestamp_ms = Arc::clone(&self.start_timestamp_ms);

        let mut stream = SCStream::new(&filter, &config);
        stream.add_output_handler(
            move |sample: CMSampleBuffer, of_type: SCStreamOutputType| {
                if of_type != SCStreamOutputType::Audio {
                    return;
                }
                if !is_recording.load(Ordering::SeqCst) {
                    return;
                }

                let mut i16_samples = convert_sck_sample_to_i16(&sample);
                if i16_samples.is_empty() {
                    return;
                }

                if let Some(ref mut wav) = *writer_clone.lock() {
                    for &sample in &i16_samples {
                        let _ = wav.write_sample(sample);
                    }
                    *samples_collected.lock() += i16_samples.len();
                }

                if let Some(ref tx) = event_tx {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    let start_ts = start_timestamp_ms.load(Ordering::SeqCst);
                    let timestamp_ms = now.saturating_sub(start_ts);

                    let _ = tx.try_send(StreamingEvent::ChunkReceived {
                        chunk: AudioChunk {
                            sample_count: i16_samples.len(),
                            samples: std::mem::take(&mut i16_samples),
                            timestamp_ms,
                        },
                    });
                }
            },
            SCStreamOutputType::Audio,
        );

        stream
            .start_capture()
            .map_err(|e| format!("Failed to start ScreenCaptureKit stream: {e}"))?;

        self.writer = Some(writer);
        self.sck_stream = Some(stream);
        log::info!("✅ Audio capture started via ScreenCaptureKit");
        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn start_wasapi_loopback(&mut self) -> Result<(), String> {
        let temp_file_path = self.create_output_file()?;

        self.is_recording.store(true, Ordering::SeqCst);
        self.start_time = Some(Instant::now());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.start_timestamp_ms.store(now, Ordering::SeqCst);
        *self.samples_collected.lock() = 0;
        self.temp_file_path = Some(temp_file_path);

        if let Some(ref tx) = self.event_tx {
            let _ = tx.try_send(StreamingEvent::Started {
                sample_rate: self.config.sample_rate,
                channels: self.config.channels,
            });
        }

        let writer = self.create_wav_writer(self.temp_file_path.as_ref().unwrap())?;
        let writer = Arc::new(Mutex::new(Some(writer)));

        let is_recording = Arc::clone(&self.is_recording);
        let samples_collected = Arc::clone(&self.samples_collected);
        let writer_clone = Arc::clone(&writer);
        let event_tx = self.event_tx.clone();
        let start_timestamp_ms = Arc::clone(&self.start_timestamp_ms);
        let sample_rate = self.config.sample_rate;
        let channels = self.config.channels;

        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let handle = std::thread::spawn(move || {
            if let Err(err) = run_wasapi_loopback_capture(
                sample_rate,
                channels,
                is_recording,
                samples_collected,
                writer_clone,
                event_tx.clone(),
                start_timestamp_ms,
                stop_rx,
            ) {
                log::error!("❌ WASAPI loopback error: {}", err);
                if let Some(tx) = event_tx {
                    let _ = tx.try_send(StreamingEvent::Error { message: err });
                }
            }
        });

        self.writer = Some(writer);
        self.wasapi_stop_tx = Some(stop_tx);
        self.wasapi_thread = Some(handle);

        log::info!("✅ Audio capture started via WASAPI loopback");
        Ok(())
    }

    /// Builds appropriate stream based on device sample format
    fn build_stream(
        &self,
        device: &Device,
        config: &StreamConfig,
        sample_format: cpal::SampleFormat,
        is_recording: Arc<AtomicBool>,
        samples_collected: Arc<Mutex<usize>>,
        writer: Arc<Mutex<Option<WavWriter<std::io::BufWriter<File>>>>>,
        event_tx: Option<crossbeam_channel::Sender<StreamingEvent>>,
        start_timestamp_ms: Arc<AtomicU64>,
    ) -> Result<Stream, String> {
        match sample_format {
            cpal::SampleFormat::I16 => {
                self.build_stream_i16(device, config, is_recording, samples_collected, writer, event_tx, start_timestamp_ms)
            }
            cpal::SampleFormat::U16 => {
                self.build_stream_u16(device, config, is_recording, samples_collected, writer, event_tx, start_timestamp_ms)
            }
            cpal::SampleFormat::F32 => {
                self.build_stream_f32(device, config, is_recording, samples_collected, writer, event_tx, start_timestamp_ms)
            }
        }
    }

    /// Builds I16 input stream and handles sample writing
    fn build_stream_i16(
        &self,
        device: &Device,
        config: &StreamConfig,
        is_recording: Arc<AtomicBool>,
        samples_collected: Arc<Mutex<usize>>,
        writer: Arc<Mutex<Option<WavWriter<std::io::BufWriter<File>>>>>,
        event_tx: Option<crossbeam_channel::Sender<StreamingEvent>>,
        start_timestamp_ms: Arc<AtomicU64>,
    ) -> Result<Stream, String> {
        let callback = move |samples: &[i16], _: &cpal::InputCallbackInfo| {
            if !is_recording.load(Ordering::SeqCst) {
                return;
            }

            let mut w = writer.lock();
            if let Some(ref mut writer) = *w {
                for &sample in samples {
                    let _ = writer.write_sample(sample);
                }
                *samples_collected.lock() += samples.len();
            }

            // Emit audio chunk event for streaming
            if let Some(ref tx) = event_tx {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let start_ts = start_timestamp_ms.load(Ordering::SeqCst);
                let timestamp_ms = now.saturating_sub(start_ts);

                let chunk = AudioChunk {
                    samples: samples.to_vec(),
                    sample_count: samples.len(),
                    timestamp_ms,
                };

                let _ = tx.try_send(StreamingEvent::ChunkReceived { chunk });
            }
        };

        let err_callback = |err| {
            log::error!("❌ Stream error: {}", err);
        };

        device
            .build_input_stream(config, callback, err_callback)
            .map_err(|e| format!("Failed to build I16 stream: {}", e))
    }

    /// Builds U16 input stream with U16 → I16 conversion
    fn build_stream_u16(
        &self,
        device: &Device,
        config: &StreamConfig,
        is_recording: Arc<AtomicBool>,
        samples_collected: Arc<Mutex<usize>>,
        writer: Arc<Mutex<Option<WavWriter<std::io::BufWriter<File>>>>>,
        event_tx: Option<crossbeam_channel::Sender<StreamingEvent>>,
        start_timestamp_ms: Arc<AtomicU64>,
    ) -> Result<Stream, String> {
        let callback = move |samples: &[u16], _: &cpal::InputCallbackInfo| {
            if !is_recording.load(Ordering::SeqCst) {
                return;
            }

            let i16_samples: Vec<i16> = samples
                .iter()
                .map(|&sample| ((sample as i32) - 32768) as i16)
                .collect();

            let mut w = writer.lock();
            if let Some(ref mut writer) = *w {
                for &i16_sample in &i16_samples {
                    let _ = writer.write_sample(i16_sample);
                }
                *samples_collected.lock() += i16_samples.len();
            }

            // Emit audio chunk event for streaming
            if let Some(ref tx) = event_tx {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let start_ts = start_timestamp_ms.load(Ordering::SeqCst);
                let timestamp_ms = now.saturating_sub(start_ts);

                let chunk = AudioChunk {
                    samples: i16_samples,
                    sample_count: samples.len(),
                    timestamp_ms,
                };

                let _ = tx.try_send(StreamingEvent::ChunkReceived { chunk });
            }
        };

        let err_callback = |err| {
            log::error!("❌ Stream error: {}", err);
        };

        device
            .build_input_stream(config, callback, err_callback)
            .map_err(|e| format!("Failed to build U16 stream: {}", e))
    }

    /// Builds F32 input stream with F32 → I16 conversion
    fn build_stream_f32(
        &self,
        device: &Device,
        config: &StreamConfig,
        is_recording: Arc<AtomicBool>,
        samples_collected: Arc<Mutex<usize>>,
        writer: Arc<Mutex<Option<WavWriter<std::io::BufWriter<File>>>>>,
        event_tx: Option<crossbeam_channel::Sender<StreamingEvent>>,
        start_timestamp_ms: Arc<AtomicU64>,
    ) -> Result<Stream, String> {
        let callback = move |samples: &[f32], _: &cpal::InputCallbackInfo| {
            if !is_recording.load(Ordering::SeqCst) {
                return;
            }

            let i16_samples: Vec<i16> = samples
                .iter()
                .map(|&sample| (sample * 32767.0) as i16)
                .collect();

            let mut w = writer.lock();
            if let Some(ref mut writer) = *w {
                for &i16_sample in &i16_samples {
                    let _ = writer.write_sample(i16_sample);
                }
                *samples_collected.lock() += i16_samples.len();
            }

            // Emit audio chunk event for streaming
            if let Some(ref tx) = event_tx {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let start_ts = start_timestamp_ms.load(Ordering::SeqCst);
                let timestamp_ms = now.saturating_sub(start_ts);

                let chunk = AudioChunk {
                    samples: i16_samples,
                    sample_count: samples.len(),
                    timestamp_ms,
                };

                let _ = tx.try_send(StreamingEvent::ChunkReceived { chunk });
            }
        };

        let err_callback = |err| {
            log::error!("❌ Stream error: {}", err);
        };

        device
            .build_input_stream(config, callback, err_callback)
            .map_err(|e| format!("Failed to build F32 stream: {}", e))
    }
}

impl Drop for AudioCapture {
    /// Ensures recording is stopped if instance is dropped while active.
    fn drop(&mut self) {
        if self.is_recording.load(Ordering::SeqCst) {
            log::warn!("⚠️ Stopping abandoned audio capture");
            let _ = self.stop();
        }
    }
}

fn score_loopback_device_name(name: &str) -> i32 {
    let lower = name.to_lowercase();
    let mut score = 0;

    // Strong loopback indicators
    if lower.contains("blackhole") {
        score += 120;
    }
    if lower.contains("vb-cable") || lower.contains("cable output") {
        score += 110;
    }
    if lower.contains("stereo mix") || lower.contains("what u hear") {
        score += 100;
    }
    if lower.contains("loopback") {
        score += 90;
    }

    // Helpful aliases often used for aggregate/multi-output routing
    if lower.contains("multi-output") || lower.contains("aggregate") {
        score += 25;
    }

    // De-prioritize obvious physical microphone labels
    if lower.contains("mic") || lower.contains("microphone") || lower.contains("headset") {
        score -= 60;
    }

    score
}

#[cfg(target_os = "macos")]
fn convert_sck_sample_to_i16(sample: &CMSampleBuffer) -> Vec<i16> {
    let Some(buffers) = sample.audio_buffer_list() else {
        return Vec::new();
    };

    let bytes_per_sample = sample.sample_size(0);
    convert_sck_audio_buffers(&buffers, bytes_per_sample)
}

#[cfg(target_os = "macos")]
fn convert_sck_audio_buffers(buffers: &AudioBufferList, bytes_per_sample: usize) -> Vec<i16> {
    if buffers.num_buffers() == 0 {
        return Vec::new();
    }

    if buffers.num_buffers() == 1 {
        if let Some(buffer) = buffers.get(0) {
            return decode_sck_pcm_to_i16(buffer.data(), bytes_per_sample);
        }
        return Vec::new();
    }

    let mut per_channel: Vec<Vec<i16>> = Vec::new();
    for buffer in buffers {
        let samples = decode_sck_pcm_to_i16(buffer.data(), bytes_per_sample);
        if !samples.is_empty() {
            per_channel.push(samples);
        }
    }

    if per_channel.is_empty() {
        return Vec::new();
    }

    let min_len = per_channel.iter().map(|ch| ch.len()).min().unwrap_or(0);
    let mut interleaved = Vec::with_capacity(min_len * per_channel.len());
    for idx in 0..min_len {
        for ch in &per_channel {
            interleaved.push(ch[idx]);
        }
    }

    interleaved
}

#[cfg(target_os = "macos")]
fn decode_sck_pcm_to_i16(bytes: &[u8], bytes_per_sample: usize) -> Vec<i16> {
    if bytes.is_empty() {
        return Vec::new();
    }

    // ScreenCaptureKit commonly returns float32 PCM. Fall back to i16 when needed.
    if (bytes_per_sample >= 4 || bytes_per_sample == 0) && bytes.len() >= 4 {
        let mut out = Vec::with_capacity(bytes.len() / 4);
        for chunk in bytes.chunks_exact(4) {
            let f = f32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            let v = (f.clamp(-1.0, 1.0) * 32767.0) as i16;
            out.push(v);
        }
        if !out.is_empty() {
            return out;
        }
    }

    let mut out = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        out.push(i16::from_ne_bytes([chunk[0], chunk[1]]));
    }
    out
}

#[cfg(target_os = "windows")]
fn run_wasapi_loopback_capture(
    sample_rate: u32,
    channels: u16,
    is_recording: Arc<AtomicBool>,
    samples_collected: Arc<Mutex<usize>>,
    writer: Arc<Mutex<Option<WavWriter<std::io::BufWriter<File>>>>>,
    event_tx: Option<crossbeam_channel::Sender<StreamingEvent>>,
    start_timestamp_ms: Arc<AtomicU64>,
    stop_rx: mpsc::Receiver<()>,
) -> Result<(), String> {
    let _ = initialize_mta();

    let enumerator = DeviceEnumerator::new().map_err(|e| format!("WASAPI enumerator error: {e}"))?;
    let device = enumerator
        .get_default_device(&Direction::Render)
        .map_err(|e| format!("Cannot get default render device: {e}"))?;

    if let Ok(name) = device.get_friendlyname() {
        log::info!("  WASAPI render endpoint: {}", name);
    }

    let mut audio_client = device
        .get_iaudioclient()
        .map_err(|e| format!("Cannot create WASAPI audio client: {e}"))?;

    let desired_format = WaveFormat::new(
        32,
        32,
        &SampleType::Float,
        sample_rate as usize,
        channels as usize,
        None,
    );
    let (_def_time, min_time) = audio_client
        .get_device_period()
        .map_err(|e| format!("Cannot get WASAPI device period: {e}"))?;
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: min_time,
    };

    audio_client
        .initialize_client(&desired_format, &Direction::Capture, &mode)
        .map_err(|e| format!("WASAPI initialize failed: {e}"))?;

    let event_handle = audio_client
        .set_get_eventhandle()
        .map_err(|e| format!("WASAPI event handle setup failed: {e}"))?;
    let capture_client = audio_client
        .get_audiocaptureclient()
        .map_err(|e| format!("WASAPI capture client failed: {e}"))?;

    audio_client
        .start_stream()
        .map_err(|e| format!("WASAPI start stream failed: {e}"))?;

    let mut byte_queue: VecDeque<u8> = VecDeque::new();

    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        if let Err(err) = capture_client.read_from_device_to_deque(&mut byte_queue) {
            if stop_rx.try_recv().is_ok() {
                break;
            }
            return Err(format!("WASAPI capture read failed: {err}"));
        }

        if !byte_queue.is_empty() {
            let chunk_bytes: Vec<u8> = byte_queue.drain(..).collect();
            let i16_samples = decode_wasapi_f32_to_i16(&chunk_bytes);
            let chunk_sample_count = i16_samples.len();

            if !i16_samples.is_empty() {
                if let Some(ref mut wav) = *writer.lock() {
                    for &sample in &i16_samples {
                        let _ = wav.write_sample(sample);
                    }
                    *samples_collected.lock() += i16_samples.len();
                }

                if let Some(ref tx) = event_tx {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    let start_ts = start_timestamp_ms.load(Ordering::SeqCst);
                    let timestamp_ms = now.saturating_sub(start_ts);

                    let _ = tx.try_send(StreamingEvent::ChunkReceived {
                        chunk: AudioChunk {
                            samples: i16_samples,
                            sample_count: chunk_sample_count,
                            timestamp_ms,
                        },
                    });
                }
            }
        }

        let _ = event_handle.wait_for_event(200);
    }

    let _ = audio_client.stop_stream();
    is_recording.store(false, Ordering::SeqCst);

    Ok(())
}

#[cfg(target_os = "windows")]
fn decode_wasapi_f32_to_i16(bytes: &[u8]) -> Vec<i16> {
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        let sample = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        out.push((sample.clamp(-1.0, 1.0) * 32767.0) as i16);
    }
    out
}
