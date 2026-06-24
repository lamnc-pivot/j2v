use std::path::PathBuf;

const MODELS_DIR_NAME: &str = ".j2v_models";
const FALLBACK_MODELS_DIR: &str = "./.models";

pub fn get_models_dir() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(MODELS_DIR_NAME))
        .unwrap_or_else(|| PathBuf::from(FALLBACK_MODELS_DIR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_models_dir_is_valid() {
        let dir = get_models_dir();
        assert!(!dir.as_os_str().is_empty());
    }
}
