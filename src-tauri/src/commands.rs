use crate::audio::{self, AudioCaptureConfig, AudioDevice, CaptureBackend, DeviceManager, StreamingEvent};
use crate::models::checker;
use crate::models::installer;
use crate::models::{ModelId, ModelStatus};
use crate::transcription;
use crossbeam_channel;
use parking_lot::Mutex;
use std::cell::RefCell;

thread_local! {
    /// Thread-local storage for audio capture instance.
    ///
    /// Audio capture requires ownership of platform-specific resources (cpal::Stream)
    /// that don't implement Send+Sync. Using thread-local storage avoids the need
    /// for an Arc<Mutex>, since each Tauri command runs on its own thread.
    static AUDIO_CAPTURE: RefCell<Option<audio::AudioCapture>> = RefCell::new(None);
}

/// Shared receiver for streaming audio events.
///
/// This must be process-wide so `get_streaming_events` can read the same channel
/// regardless of which worker thread executes the Tauri command.
static STREAM_EVENT_RX: Mutex<Option<crossbeam_channel::Receiver<StreamingEvent>>> =
    Mutex::new(None);

#[tauri::command]
pub fn check_model_status() -> Result<Vec<ModelStatus>, String> {
    log::info!("📋 Checking model status");
    Ok(checker::check_all_models())
}

#[tauri::command]
pub fn install_model(model_id: &str) -> Result<String, String> {
    log::info!("⬇️ Installing model: {}", model_id);

    let model =
        ModelId::from_str(model_id).ok_or_else(|| format!("Unknown model: {}", model_id))?;

    installer::install_model(model)
        .map(|result| {
            log::info!("✅ Installed {}", model_id);
            result
        })
        .map_err(|e| {
            log::error!("❌ Failed to install {}: {}", model_id, e);
            e.to_string()
        })
}

/// Starts audio capture from the default input device.
///
/// # Returns
/// A success message if recording begins, error message otherwise.
#[tauri::command]
pub fn start_audio_capture(input_device_name: Option<String>) -> Result<String, String> {
    log::info!("🎙️ Audio capture start requested");

    AUDIO_CAPTURE.with(|capture| {
        let mut cap_ref = capture.borrow_mut();

        if let Some(ref c) = *cap_ref {
            if c.is_recording() {
                return Err("Already recording".to_string());
            }
        }

        let mut config = AudioCaptureConfig::default();
        config.input_device_name = input_device_name;
        let mut audio_capture = audio::AudioCapture::new(config);

        audio_capture.start()?;
        *cap_ref = Some(audio_capture);

        Ok("Audio capture started".to_string())
    })
}

/// Stops audio capture and returns recording metadata.
///
/// # Returns
/// Audio data including duration, sample count, and file path.
#[tauri::command]
pub fn stop_audio_capture() -> Result<audio::AudioData, String> {
    log::info!("⏹️ Audio capture stop requested");

    AUDIO_CAPTURE.with(|capture| {
        let mut cap_ref = capture.borrow_mut();

        let mut audio_capture = cap_ref
            .take()
            .ok_or_else(|| "No active recording".to_string())?;

        if !audio_capture.is_recording() {
            return Err("Not currently recording".to_string());
        }

        audio_capture.stop()
    })
}

/// Lists all available audio input devices.
#[tauri::command]
pub fn list_audio_devices() -> Result<Vec<AudioDevice>, String> {
    log::info!("📻 Listing audio devices");
    DeviceManager::list_input_devices()
}

/// Gets current audio capture status.
#[tauri::command]
pub fn get_audio_capture_status() -> Result<audio::CaptureStatus, String> {
    AUDIO_CAPTURE.with(|capture| {
        let cap_ref = capture.borrow();

        match cap_ref.as_ref() {
            Some(c) => {
                let is_recording = c.is_recording();
                let duration = c.get_duration();

                Ok(audio::CaptureStatus {
                    is_recording,
                    device_name: Some("System Audio (Loopback)".to_string()),
                    sample_rate: Some(48000),
                    duration_secs: duration,
                })
            }
            None => Ok(audio::CaptureStatus {
                is_recording: false,
                device_name: None,
                sample_rate: None,
                duration_secs: 0.0,
            }),
        }
    })
}

/// Starts streaming audio capture with real-time event emission.
///
/// Creates a channel for audio chunk events and begins recording.
/// Events are retrieved via `get_streaming_events`.
///
/// # Returns
/// A success message if recording begins, error message otherwise.
#[tauri::command]
pub fn start_streaming_capture(
    input_device_name: Option<String>,
    capture_backend: Option<String>,
) -> Result<String, String> {
    log::info!("🎙️ Streaming capture start requested");

    AUDIO_CAPTURE.with(|capture| {
        let mut cap_ref = capture.borrow_mut();

        if let Some(ref c) = *cap_ref {
            if c.is_recording() {
                return Err("Already recording".to_string());
            }
        }

        // Create event channel
        let (tx, rx) = crossbeam_channel::unbounded();

        let mut config = AudioCaptureConfig::default();
        config.input_device_name = input_device_name;
        config.capture_backend = parse_capture_backend(capture_backend);
        let mut audio_capture = audio::AudioCapture::new(config);
        
        // Set up event channel
        audio_capture.set_event_channel(tx);

        // Start recording
        audio_capture.start()?;
        
        *cap_ref = Some(audio_capture);
        *STREAM_EVENT_RX.lock() = Some(rx);

        log::info!("✅ Streaming capture started");
        Ok("Streaming capture started".to_string())
    })
}

/// Starts streaming audio capture with real-time transcription via Whisper.
///
/// When a faster-whisper model is available, a background worker buffers audio chunks,
/// downsamples them, and calls the Python transcription script every few seconds.
/// Transcription results arrive as `TranscriptionReady` events via `get_streaming_events`.
///
/// # Returns
/// A success message if recording begins, error message otherwise.
#[tauri::command]
pub fn start_streaming_capture_with_transcription(
    input_device_name: Option<String>,
    capture_backend: Option<String>,
) -> Result<String, String> {
    log::info!("🎙️ Streaming capture + transcription start requested");

    AUDIO_CAPTURE.with(|capture| {
        let mut cap_ref = capture.borrow_mut();

        if let Some(ref c) = *cap_ref {
            if c.is_recording() {
                return Err("Already recording".to_string());
            }
        }

        // Locate transcription assets
        let script_path = transcription::get_script_path()
            .ok_or_else(|| "transcribe.py not found – check app installation".to_string())?;

        let model_dir = crate::utils::paths::get_models_dir().join("faster-whisper");
        validate_whisper_model_dir(&model_dir)?;

        // Transcription channel: worker → result_tx → result_rx (polled by frontend)
        let (result_tx, result_rx) = crossbeam_channel::unbounded::<StreamingEvent>();

        // We need the frontend to receive BOTH audio events AND transcription events.
        // Solve this by a fan-out relay thread: audio_tx is cloned so both the relay
        // thread and the capture write to the same channel, and the relay also forwards
        // audio events to the transcription worker.
        //
        // Simpler approach: give audio capture its own channel, spawn a relay that
        // forwards every event to BOTH the public result channel and the worker channel.
        let (relay_worker_tx, relay_worker_rx) = crossbeam_channel::unbounded::<StreamingEvent>();
        let (relay_in_tx, relay_in_rx) = crossbeam_channel::unbounded::<StreamingEvent>();
        let relay_result_tx = result_tx.clone();

        // Relay thread: reads from audio capture channel, fans out to frontend + worker
        std::thread::spawn(move || {
            while let Ok(event) = relay_in_rx.recv() {
                let _ = relay_result_tx.send(event.clone());
                let _ = relay_worker_tx.send(event);
            }
            log::debug!("Relay thread exiting");
        });

        // Audio capture writes into relay_in_tx
        let mut config = AudioCaptureConfig::default();
        config.input_device_name = input_device_name;
        config.capture_backend = parse_capture_backend(capture_backend);
        let mut audio_capture = audio::AudioCapture::new(config);
        audio_capture.set_event_channel(relay_in_tx);
        audio_capture.start()?;

        let capture_sample_rate = audio_capture.sample_rate();
        let capture_channels = audio_capture.channels();

        log::info!(
            "Starting transcription worker with capture format: {}Hz, {}ch",
            capture_sample_rate,
            capture_channels
        );

        // Spawn transcription worker – reads from relay_worker_rx, emits to result_tx
        transcription::spawn_transcription_worker(
            relay_worker_rx,
            result_tx,
            capture_sample_rate,
            capture_channels,
            model_dir,
            script_path,
        );

        *cap_ref = Some(audio_capture);
        *STREAM_EVENT_RX.lock() = Some(result_rx);

        log::info!("✅ Streaming capture + transcription started");
        Ok("Streaming capture with transcription started".to_string())
    })
}

/// Transcribes a WAV audio file using faster-whisper.
///
/// Useful for post-recording transcription. The file should be a valid WAV.
/// It will be automatically resampled to 16 kHz mono if needed by the Python script.
///
/// # Arguments
/// * `file_path` – Absolute path to the WAV file
///
/// # Returns
/// The transcribed Japanese text, or an error message.
#[tauri::command]
pub fn transcribe_audio_file(file_path: String) -> Result<String, String> {
    log::info!("📝 Transcribe request: {}", file_path);

    let wav_path = std::path::PathBuf::from(&file_path);
    if !wav_path.exists() {
        return Err(format!("File not found: {file_path}"));
    }

    let script_path = transcription::get_script_path()
        .ok_or_else(|| "transcribe.py not found – check app installation".to_string())?;

    let model_dir = crate::utils::paths::get_models_dir().join("faster-whisper");
    validate_whisper_model_dir(&model_dir)?;

    let result = transcription::transcribe_wav(&wav_path, &model_dir, &script_path)?;
    log::info!(
        "✅ Transcription done (lang={}, p={:.2}): {}",
        result.language,
        result.language_probability,
        result.text
    );
    Ok(result.text)
}

fn validate_whisper_model_dir(model_dir: &std::path::Path) -> Result<(), String> {
    if !model_dir.exists() {
        return Err(
            "faster-whisper model not installed. Please install it from the Models screen."
                .to_string(),
        );
    }

    let direct_valid =
        model_dir.join("model.bin").exists() && model_dir.join("config.json").exists();
    let nested_valid = std::fs::read_dir(model_dir)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .map(|entry| entry.path())
        .any(|path| {
            path.is_dir() && path.join("model.bin").exists() && path.join("config.json").exists()
        });

    if !(direct_valid || nested_valid) {
        return Err(
            "faster-whisper model files not found. Please install the model from the Models screen."
                .to_string(),
        );
    }

    Ok(())
}

fn parse_capture_backend(value: Option<String>) -> CaptureBackend {
    #[cfg(target_os = "macos")]
    if value.is_none() {
        return CaptureBackend::ScreenCaptureKit;
    }

    match value.unwrap_or_else(|| "auto".to_string()).as_str() {
        "screen-capture-kit" => CaptureBackend::ScreenCaptureKit,
        "wasapi-loopback" => CaptureBackend::WasapiLoopback,
        "loopback-driver" => CaptureBackend::LoopbackDriver,
        _ => CaptureBackend::Auto,
    }
}
/// Stops streaming audio capture.
///
/// # Returns
/// Final audio data including duration, sample count, and file path.
#[tauri::command]
pub fn stop_streaming_capture() -> Result<audio::AudioData, String> {
    log::info!("⏹️ Streaming capture stop requested");

    AUDIO_CAPTURE.with(|capture| {
        let mut cap_ref = capture.borrow_mut();

        let mut audio_capture = cap_ref
            .take()
            .ok_or_else(|| "No active recording".to_string())?;

        if !audio_capture.is_recording() {
            return Err("Not currently recording".to_string());
        }

        let result = audio_capture.stop();

        // Clean up event channel
        *STREAM_EVENT_RX.lock() = None;

        result
    })
}

/// Retrieves pending streaming audio events.
///
/// Call this repeatedly to get audio chunks and status events in real-time.
/// Returns up to the specified number of events in a single batch.
///
/// # Arguments
/// * `max_events` - Maximum number of events to return per call
///
/// # Returns
/// A vector of streaming events (may be empty if no events pending)
#[tauri::command]
pub fn get_streaming_events(max_events: usize) -> Vec<StreamingEvent> {
    let rx_guard = STREAM_EVENT_RX.lock();

    if let Some(ref receiver) = *rx_guard {
        let mut events = Vec::with_capacity(max_events);

        // Try to collect up to max_events without blocking
        for _ in 0..max_events {
            match receiver.try_recv() {
                Ok(event) => events.push(event),
                Err(_) => break, // Channel empty or disconnected
            }
        }

        events
    } else {
        Vec::new()
    }
}
