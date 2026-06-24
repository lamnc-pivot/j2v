//! Whisper speech-to-text transcription module.
//!
//! Calls the faster-whisper Python script as a subprocess,
//! passing a WAV file and model directory, returning transcription text.

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Result from the faster-whisper Python subprocess
#[derive(Debug, Deserialize)]
struct WhisperOutput {
    success: bool,
    text: Option<String>,
    language: Option<String>,
    language_probability: Option<f32>,
    error: Option<String>,
}

/// Transcription result returned to callers
#[derive(Debug, Clone)]
pub struct TranscriptionResult {
    pub text: String,
    pub language: String,
    pub language_probability: f32,
}

/// Calls faster-whisper via Python subprocess to transcribe a WAV file.
///
/// # Arguments
/// * `audio_path` – Path to the 16kHz mono WAV file to transcribe
/// * `model_dir`  – Path to the faster-whisper model directory
/// * `script_path` – Path to the `transcribe.py` Python script
///
/// # Returns
/// `Ok(TranscriptionResult)` on success, `Err(String)` with a human-readable message on failure.
pub fn transcribe_wav(
    audio_path: &Path,
    model_dir: &Path,
    script_path: &Path,
) -> Result<TranscriptionResult, String> {
    log::debug!(
        "🎤 Transcribing: {} (model: {})",
        audio_path.display(),
        model_dir.display()
    );

    // Find a usable Python executable
    let python = find_python()?;

    let output = Command::new(&python)
        .arg(script_path)
        .arg(audio_path)
        .arg(model_dir)
        .output()
        .map_err(|e| format!("Failed to launch Python ({python}): {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Also try to parse stdout in case there's a JSON error message
        if let Ok(stdout_str) = std::str::from_utf8(&output.stdout) {
            if let Ok(parsed) = serde_json::from_str::<WhisperOutput>(stdout_str) {
                if let Some(err_msg) = parsed.error {
                    return Err(err_msg);
                }
            }
        }
        return Err(format!("Whisper process failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: WhisperOutput = serde_json::from_str(&stdout)
        .map_err(|e| format!("Failed to parse whisper output: {e}\nOutput: {stdout}"))?;

    if !parsed.success {
        return Err(parsed.error.unwrap_or_else(|| "Unknown whisper error".to_string()));
    }

    Ok(TranscriptionResult {
        text: parsed.text.unwrap_or_default(),
        language: parsed.language.unwrap_or_else(|| "ja".to_string()),
        language_probability: parsed.language_probability.unwrap_or(0.0),
    })
}

/// Finds a usable Python 3 executable on the system.
fn find_python() -> Result<String, String> {
    for candidate in &["python3", "python"] {
        if Command::new(candidate)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Ok(candidate.to_string());
        }
    }
    Err("Python 3 not found. Please install Python 3.8+".to_string())
}

/// Returns the expected path to the transcribe.py script bundled with the app.
///
/// Looks in:
/// 1. Tauri resource directory (production)
/// 2. `../../scripts/` relative to the binary (dev)
pub fn get_script_path() -> Option<PathBuf> {
    // Dev: script lives in <project root>/scripts/transcribe.py
    // Walk up from the binary to find it
    if let Ok(exe) = std::env::current_exe() {
        // In dev mode the binary is in target/debug/
        // Walk up max 5 levels to find scripts/transcribe.py
        let mut dir = exe.as_path();
        for _ in 0..6 {
            let candidate = dir.join("scripts").join("transcribe.py");
            if candidate.exists() {
                log::debug!("Found transcribe.py at {}", candidate.display());
                return Some(candidate);
            }
            if let Some(parent) = dir.parent() {
                dir = parent;
            } else {
                break;
            }
        }
    }
    None
}
