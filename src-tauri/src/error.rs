use serde::Serialize;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Serialize)]
pub enum AppError {
    #[serde(rename = "io_error")]
    IoError(String),
    #[serde(rename = "installation_error")]
    InstallationError(String),
    #[serde(rename = "not_supported")]
    #[allow(dead_code)]
    NotSupported(String),
    #[serde(rename = "invalid_model")]
    #[allow(dead_code)]
    InvalidModel(String),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IoError(e) => write!(f, "IO Error: {}", e),
            Self::InstallationError(e) => write!(f, "Installation Error: {}", e),
            Self::NotSupported(e) => write!(f, "Not Supported: {}", e),
            Self::InvalidModel(e) => write!(f, "Invalid Model: {}", e),
        }
    }
}

impl std::error::Error for AppError {}
