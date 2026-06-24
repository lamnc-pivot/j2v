use std::path::PathBuf;
use std::process::Command;

use super::ModelId;
use crate::error::{AppError, AppResult};
use crate::utils::paths::get_models_dir;

const QWEN_MODEL_TAG: &str = "qwen2.5:7b";
const SILERO_VAD_ASSET_FILE: &str = "silero_vad_v6.onnx";
const SILERO_VAD_DOWNLOAD_URL: &str = "https://raw.githubusercontent.com/SYSTRAN/faster-whisper/master/faster_whisper/assets/silero_vad_v6.onnx";

pub fn install_model(model_id: ModelId) -> AppResult<String> {
    log::info!("📥 Starting installation for {}", model_id.display_name());

    ensure_models_dir_exists()?;

    match model_id {
        ModelId::Ollama => install_ollama(),
        ModelId::Qwen => install_qwen_model(),
        ModelId::FasterWhisper => install_faster_whisper(),
        ModelId::SileroVad => install_silero_vad(),
        ModelId::MeloTts => install_generic_model(model_id, "melotts"),
    }
}

fn ensure_models_dir_exists() -> AppResult<()> {
    let models_dir = get_models_dir();
    log::debug!("Ensuring models directory exists: {:?}", models_dir);

    std::fs::create_dir_all(&models_dir).map_err(|e| {
        log::error!("Failed to create models directory: {}", e);
        AppError::IoError(format!("Failed to create models directory: {}", e))
    })?;

    log::debug!("Models directory ready");
    Ok(())
}

fn install_ollama() -> AppResult<String> {
    log::info!("Installing Ollama...");

    #[cfg(target_os = "macos")]
    {
        return install_ollama_macos();
    }

    #[cfg(target_os = "windows")]
    {
        let msg = "Please download Ollama from https://ollama.ai and install manually";
        log::warn!("{}", msg);
        return Err(AppError::NotSupported(msg.to_string()));
    }

    #[cfg(target_os = "linux")]
    {
        return install_ollama_linux();
    }

    #[allow(unreachable_code)]
    Err(AppError::NotSupported(
        "Ollama installation is not supported on this OS".to_string(),
    ))
}

#[cfg(target_os = "macos")]
fn install_ollama_macos() -> AppResult<String> {
    log::debug!("Running: brew install ollama");

    let output = Command::new("brew")
        .args(&["install", "ollama"])
        .output()
        .map_err(|e| {
            log::error!("Failed to execute brew: {}", e);
            AppError::InstallationError(format!("Failed to install Ollama: {}", e))
        })?;

    if output.status.success() {
        log::info!("✅ Ollama installation successful");
        Ok("Ollama installed successfully".to_string())
    } else {
        let error_msg = String::from_utf8_lossy(&output.stderr);
        log::error!("Brew install failed: {}", error_msg);
        Err(AppError::InstallationError(format!(
            "Ollama installation failed: {}",
            error_msg
        )))
    }
}

#[cfg(target_os = "linux")]
fn install_ollama_linux() -> AppResult<String> {
    log::debug!("Running: curl -fsSL https://ollama.ai/install.sh | sh");

    let output = Command::new("sh")
        .arg("-c")
        .arg("curl -fsSL https://ollama.ai/install.sh | sh")
        .output()
        .map_err(|e| {
            log::error!("Failed to execute install script: {}", e);
            AppError::InstallationError(format!("Failed to install Ollama: {}", e))
        })?;

    if output.status.success() {
        log::info!("✅ Ollama installation successful");
        Ok("Ollama installed successfully".to_string())
    } else {
        let error_msg = String::from_utf8_lossy(&output.stderr);
        log::error!("Installation script failed: {}", error_msg);
        Err(AppError::InstallationError(format!(
            "Ollama installation failed: {}",
            error_msg
        )))
    }
}

fn install_generic_model(model_id: ModelId, directory_name: &str) -> AppResult<String> {
    log::debug!("Installing generic model: {}", directory_name);

    let models_dir = get_models_dir();
    let model_dir = models_dir.join(directory_name);

    log::debug!("Creating directory: {:?}", model_dir);
    std::fs::create_dir_all(&model_dir).map_err(|e| {
        log::error!("Failed to create {} directory: {}", directory_name, e);
        AppError::IoError(format!(
            "Failed to create {} directory: {}",
            directory_name, e
        ))
    })?;

    mark_model_installed(&model_dir)?;

    let message = format!("{} model prepared", model_id.display_name());
    log::info!("✅ {}", message);
    Ok(message)
}

fn install_silero_vad() -> AppResult<String> {
    let python = find_python_executable()
        .ok_or_else(|| AppError::InstallationError("Python 3 not found".to_string()))?;

    let script = format!(
        r#"
import os
import sys
import urllib.request

ASSET_FILE = {asset_file:?}
DOWNLOAD_URL = {download_url:?}

try:
    from faster_whisper.utils import get_assets_path
except Exception as exc:
    sys.stderr.write(
        "faster-whisper is not installed or failed to import. Install Python dependencies first: pip install faster-whisper onnxruntime numpy\\n"
    )
    sys.stderr.write(f"Import error: {{exc}}\\n")
    raise SystemExit(1)

assets_dir = get_assets_path()
os.makedirs(assets_dir, exist_ok=True)
asset_path = os.path.join(assets_dir, ASSET_FILE)

if os.path.exists(asset_path) and os.path.getsize(asset_path) > 0:
    print(asset_path)
    raise SystemExit(0)

urllib.request.urlretrieve(DOWNLOAD_URL, asset_path)

if not os.path.exists(asset_path) or os.path.getsize(asset_path) == 0:
    sys.stderr.write(f"Downloaded file missing or empty: {{asset_path}}\\n")
    raise SystemExit(1)

print(asset_path)
"#,
        asset_file = SILERO_VAD_ASSET_FILE,
        download_url = SILERO_VAD_DOWNLOAD_URL,
    );

    let output = Command::new(&python)
        .arg("-c")
        .arg(script)
        .output()
        .map_err(|e| {
            AppError::InstallationError(format!(
                "Failed to execute Silero VAD installer with {}: {}",
                python, e
            ))
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(AppError::InstallationError(format!(
            "Silero VAD installation failed: {}",
            detail
        )));
    }

    let message = if stdout.is_empty() {
        "Silero VAD runtime model installed successfully".to_string()
    } else {
        format!(
            "Silero VAD runtime model installed successfully ({})",
            stdout
        )
    };

    log::info!("✅ {}", message);
    Ok(message)
}

fn install_qwen_model() -> AppResult<String> {
    ensure_ollama_cli_available()?;

    if is_ollama_model_installed(QWEN_MODEL_TAG)? {
        let message = format!("Qwen model already installed ({})", QWEN_MODEL_TAG);
        log::info!("✅ {}", message);
        return Ok(message);
    }

    log::info!("Pulling Qwen model via Ollama: {}", QWEN_MODEL_TAG);
    let output = run_ollama(&["pull", QWEN_MODEL_TAG])?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else {
            stdout.trim().to_string()
        };

        return Err(AppError::InstallationError(format!(
            "Failed to pull Qwen model ({}): {}",
            QWEN_MODEL_TAG, detail
        )));
    }

    if !is_ollama_model_installed(QWEN_MODEL_TAG)? {
        return Err(AppError::InstallationError(format!(
            "Qwen model pull finished but verification failed for {}",
            QWEN_MODEL_TAG
        )));
    }

    let message = format!("Qwen model installed successfully ({})", QWEN_MODEL_TAG);
    log::info!("✅ {}", message);
    Ok(message)
}

fn ensure_ollama_cli_available() -> AppResult<()> {
    let output = run_ollama(&["--version"])?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(AppError::InstallationError(format!(
        "Ollama is not available. Please install/start Ollama first: {}",
        stderr.trim()
    )))
}

fn is_ollama_model_installed(model_tag: &str) -> AppResult<bool> {
    let output = run_ollama(&["list"])?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::InstallationError(format!(
            "Failed to query Ollama model list: {}",
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_ollama_list_contains_model(&stdout, model_tag))
}

fn parse_ollama_list_contains_model(output: &str, model_tag: &str) -> bool {
    output
        .lines()
        .skip(1)
        .filter_map(|line| line.split_whitespace().next())
        .any(|name| name == model_tag)
}

fn run_ollama(args: &[&str]) -> AppResult<std::process::Output> {
    Command::new("ollama").args(args).output().map_err(|e| {
        AppError::InstallationError(format!("Failed to execute ollama {:?}: {}", args, e))
    })
}

fn mark_model_installed(model_dir: &PathBuf) -> AppResult<()> {
    let marker_file = model_dir.join(".installed");
    log::debug!("Creating marker file: {:?}", marker_file);

    std::fs::write(&marker_file, "").map_err(|e| {
        log::error!("Failed to mark installation: {}", e);
        AppError::IoError(format!("Failed to mark installation: {}", e))
    })?;

    Ok(())
}

fn install_faster_whisper() -> AppResult<String> {
    let models_dir = get_models_dir();
    let model_dir = models_dir.join("faster-whisper");
    std::fs::create_dir_all(&model_dir).map_err(|e| {
        AppError::IoError(format!("Failed to create faster-whisper directory: {}", e))
    })?;

    let script_path = find_script_path("install_whisper_model.py").ok_or_else(|| {
        AppError::InstallationError("install_whisper_model.py not found".to_string())
    })?;

    let python = find_python_executable()
        .ok_or_else(|| AppError::InstallationError("Python 3 not found".to_string()))?;

    // Default model profile for local CPU usage.
    let model_name = "small";

    let output = Command::new(&python)
        .arg(&script_path)
        .arg(model_name)
        .arg(&model_dir)
        .output()
        .map_err(|e| {
            AppError::InstallationError(format!(
                "Failed to execute Whisper installer with {}: {}",
                python, e
            ))
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        let detail = if !stderr.trim().is_empty() {
            stderr
        } else {
            stdout
        };
        return Err(AppError::InstallationError(format!(
            "Whisper model installation failed: {}",
            detail.trim()
        )));
    }

    Ok(format!(
        "Faster-Whisper model installed successfully (profile: {})",
        model_name
    ))
}

fn find_python_executable() -> Option<String> {
    for candidate in ["python3", "python"] {
        let ok = Command::new(candidate)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok {
            return Some(candidate.to_string());
        }
    }
    None
}

fn find_script_path(script_name: &str) -> Option<PathBuf> {
    if let Ok(exe_path) = std::env::current_exe() {
        let mut dir = exe_path.parent();
        for _ in 0..8 {
            if let Some(current) = dir {
                let candidate = current.join("scripts").join(script_name);
                if candidate.exists() {
                    return Some(candidate);
                }
                dir = current.parent();
            } else {
                break;
            }
        }
    }

    None
}
