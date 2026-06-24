use serde::{Deserialize, Serialize};

pub mod checker;
pub mod installer;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ModelStatus {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelId {
    Ollama,
    Qwen,
    FasterWhisper,
    SileroVad,
    MeloTts,
}

impl ModelId {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::Qwen => "qwen",
            Self::FasterWhisper => "faster-whisper",
            Self::SileroVad => "silero-vad",
            Self::MeloTts => "melotts",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Ollama => "Ollama",
            Self::Qwen => "Qwen2.5-7B",
            Self::FasterWhisper => "Faster-Whisper",
            Self::SileroVad => "Silero VAD",
            Self::MeloTts => "MeloTTS",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "ollama" => Some(Self::Ollama),
            "qwen" => Some(Self::Qwen),
            "faster-whisper" => Some(Self::FasterWhisper),
            "silero-vad" => Some(Self::SileroVad),
            "melotts" => Some(Self::MeloTts),
            _ => None,
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Ollama,
            Self::Qwen,
            Self::FasterWhisper,
            Self::SileroVad,
            Self::MeloTts,
        ]
    }
}

impl ModelStatus {
    pub fn new(model_id: ModelId, installed: bool, path: Option<String>) -> Self {
        Self {
            id: model_id.as_str().to_string(),
            name: model_id.display_name().to_string(),
            installed,
            path,
        }
    }
}
