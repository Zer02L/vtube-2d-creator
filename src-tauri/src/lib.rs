pub mod audio;
pub mod optimizer;
pub mod profile;

use optimizer::TranscodeOptions;

#[tauri::command]
fn get_obs_url() -> String {
    "http://localhost:12345/avatar".to_string()
}

#[tauri::command]
fn preview_ffmpeg_args(options: TranscodeOptions) -> Vec<String> {
    let cmd = optimizer::VideoOptimizer::build_ffmpeg_command(&options);
    cmd.get_args()
        .map(|s| s.to_string_lossy().into_owned())
        .collect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_obs_url,
            preview_ffmpeg_args
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
