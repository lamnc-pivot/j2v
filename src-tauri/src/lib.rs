mod audio;
mod commands;
mod error;
mod models;
mod transcription;
mod utils;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize logger
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .format_timestamp_secs()
        .init();

    log::info!("🚀 Starting J2V application...");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::check_model_status,
            commands::install_model,
            commands::start_audio_capture,
            commands::stop_audio_capture,
            commands::list_audio_devices,
            commands::get_audio_capture_status,
            commands::start_streaming_capture,
            commands::stop_streaming_capture,
            commands::get_streaming_events,
            commands::start_streaming_capture_with_transcription,
            commands::transcribe_audio_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
