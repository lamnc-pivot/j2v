//! Transcription module – wraps faster-whisper for Japanese speech-to-text.

pub mod buffer_worker;
pub mod whisper;
pub use whisper::{get_script_path, transcribe_wav};
pub use buffer_worker::spawn_transcription_worker;
