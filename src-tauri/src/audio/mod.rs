//! Audio capture module for recording from input devices.
//!
//! This module provides:
//! - Real-time audio capture from user-selected input device
//! - WAV file output with PCM16 encoding
//! - Multi-format support (I16, U16, F32 → PCM16 conversion)
//! - Device enumeration and status tracking

pub mod capture;
pub mod device;

pub use capture::{AudioCapture, AudioData};
pub use device::{AudioDevice, DeviceManager};

use serde::{Deserialize, Serialize};
// use std::path::PathBuf;

/// Platform-specific audio capture backend.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureBackend {
    /// Automatically choose backend by platform and availability.
    Auto,
    /// Capture from loopback driver exposed as an input device.
    LoopbackDriver,
    /// Native ScreenCaptureKit system audio capture (macOS).
    ScreenCaptureKit,
    /// Native WASAPI loopback capture (Windows).
    WasapiLoopback,
}

/// Real-time audio chunk for streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioChunk {
    /// Raw audio samples (I16 format)
    pub samples: Vec<i16>,
    /// Number of samples in this chunk
    pub sample_count: usize,
    /// Timestamp of when chunk was captured (milliseconds)
    pub timestamp_ms: u64,
}

/// Streaming events emitted during audio capture
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StreamingEvent {
    /// Recording started successfully
    Started {
        /// Sample rate in Hz
        sample_rate: u32,
        /// Number of channels
        channels: u16,
    },
    /// New audio chunk available
    ChunkReceived {
        /// The audio chunk data
        chunk: AudioChunk,
    },
    /// Recording stopped
    Stopped {
        /// Total duration in seconds
        duration_secs: f32,
        /// Total samples captured
        sample_count: usize,
        /// Path to saved WAV file
        file_path: Option<String>,
    },
    /// Error occurred during recording
    Error {
        /// Error message
        message: String,
    },
        /// Transcription result ready
        TranscriptionReady {
            /// Transcribed Japanese text
            text: String,
            /// Detected language code (e.g. "ja")
            language: String,
            /// Language detection confidence [0.0, 1.0]
            language_probability: f32,
        },
        /// Error occurred during transcription
        TranscriptionError {
            /// Error message
            message: String,
        },
}

/// Real-time status of audio capture session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureStatus {
    /// Whether recording is currently active
    pub is_recording: bool,
    /// Name of the input device in use
    pub device_name: Option<String>,
    /// Sample rate in Hz (e.g., 48000)
    pub sample_rate: Option<u32>,
    /// Elapsed recording duration in seconds
    pub duration_secs: f32,
}

/// Supported audio output format
// pub enum AudioFormat {
//     /// 16-bit signed PCM
//     #[serde(rename = "pcm16")]
//     PCM16,
// }

/// Configuration for audio capture session
// #[derive(Debug, Clone)]
pub struct AudioCaptureConfig {
    /// Sample rate in Hz (default: 48000)
    pub sample_rate: u32,
    /// Number of audio channels (default: 2)
    pub channels: u16,
    /// Optional input device name. If None, use system default input.
    pub input_device_name: Option<String>,
    /// Chosen capture backend.
    pub capture_backend: CaptureBackend,
    // Output format (default: PCM16)
    // pub format: AudioFormat,
    // Optional custom output path (default: system cache)
    // pub output_path: Option<PathBuf>,
}

impl Default for AudioCaptureConfig {
    /// Creates default configuration: 48kHz, 2-channel stereo, PCM16
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 2,
            input_device_name: None,
            capture_backend: CaptureBackend::Auto,
            // format: AudioFormat::PCM16,
            // output_path: None,
        }
    }
}
