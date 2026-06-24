use std::path::PathBuf;
use std::process::Command;

use super::{ModelId, ModelStatus};
use crate::utils::paths::get_models_dir;

const QWEN_MODEL_TAG: &str = "qwen2.5:7b";

pub fn check_all_models() -> Vec<ModelStatus> {
    log::debug!("Checking all models...");

    ModelId::all()
        .iter()
        .map(|&model_id| check_model(model_id))
        .collect()
}

pub fn check_model(model_id: ModelId) -> ModelStatus {
    log::debug!("Checking {}", model_id.display_name());

    match model_id {
        ModelId::Ollama => check_ollama(),
        ModelId::Qwen => check_qwen(),
        ModelId::FasterWhisper => check_faster_whisper(),
        ModelId::SileroVad => check_generic_model(model_id, "silero-vad"),
        ModelId::MeloTts => check_generic_model(model_id, "melotts"),
    }
}

fn check_ollama() -> ModelStatus {
    let ollama_path = get_ollama_path();
    let installed = ollama_path
        .as_ref()
        .map(|p| PathBuf::from(p).exists())
        .unwrap_or(false);

    log::debug!(
        "Ollama status: {}",
        if installed {
            "installed"
        } else {
            "not installed"
        }
    );

    ModelStatus::new(ModelId::Ollama, installed, ollama_path)
}

fn check_generic_model(model_id: ModelId, directory_name: &str) -> ModelStatus {
    let models_dir = get_models_dir();
    let model_dir = models_dir.join(directory_name);
    let installed = model_dir.exists();

    log::debug!(
        "{} status: {} (path: {:?})",
        model_id.display_name(),
        if installed {
            "installed"
        } else {
            "not installed"
        },
        model_dir
    );

    let path = model_dir.to_str().map(|s| s.to_string());

    ModelStatus::new(model_id, installed, path)
}

fn check_qwen() -> ModelStatus {
    let installed = match run_ollama(&["list"]) {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            parse_ollama_list_contains_model(&stdout, QWEN_MODEL_TAG)
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::debug!("Failed to query Ollama list for Qwen check: {}", stderr.trim());
            false
        }
        Err(error) => {
            log::debug!("Ollama unavailable during Qwen check: {}", error);
            false
        }
    };

    log::debug!(
        "{} status: {} (model: {})",
        ModelId::Qwen.display_name(),
        if installed { "installed" } else { "not installed" },
        QWEN_MODEL_TAG
    );

    let path = if installed {
        Some(format!("ollama://{}", QWEN_MODEL_TAG))
    } else {
        None
    };

    ModelStatus::new(ModelId::Qwen, installed, path)
}

fn check_faster_whisper() -> ModelStatus {
    let models_dir = get_models_dir();
    let model_dir = models_dir.join("faster-whisper");
    let installed = has_real_whisper_model(&model_dir);

    log::debug!(
        "{} status: {} (path: {:?})",
        ModelId::FasterWhisper.display_name(),
        if installed {
            "installed"
        } else {
            "not installed"
        },
        model_dir
    );

    let path = model_dir.to_str().map(|s| s.to_string());
    ModelStatus::new(ModelId::FasterWhisper, installed, path)
}

fn has_real_whisper_model(model_dir: &PathBuf) -> bool {
    if !model_dir.exists() {
        return false;
    }

    // Accept either direct model files under the folder,
    // or a nested model directory produced by some download flows.
    let direct = model_dir.join("model.bin").exists() && model_dir.join("config.json").exists();
    if direct {
        return true;
    }

    if let Ok(entries) = std::fs::read_dir(model_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir()
                && path.join("model.bin").exists()
                && path.join("config.json").exists()
            {
                return true;
            }
        }
    }

    false
}

fn get_ollama_path() -> Option<String> {
    if cfg!(target_os = "macos") {
        Some(format!("{}/.ollama", dirs::home_dir()?.display()))
    } else if cfg!(target_os = "windows") {
        Some(format!(
            "{}\\AppData\\Local\\Ollama",
            dirs::home_dir()?.display()
        ))
    } else {
        Some(format!("{}/.ollama", dirs::home_dir()?.display()))
    }
}

fn parse_ollama_list_contains_model(output: &str, model_tag: &str) -> bool {
    output
        .lines()
        .skip(1)
        .filter_map(|line| line.split_whitespace().next())
        .any(|name| name == model_tag)
}

fn run_ollama(args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("ollama")
        .args(args)
        .output()
        .map_err(|e| format!("Failed to execute ollama {:?}: {}", args, e))
}

