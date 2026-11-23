pub mod audio;
pub mod looper;
pub mod optimizer;
pub mod profile;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use looper::{
    LoopAnalysisResult, LoopProcessOptions, LoopProcessResult, LoopProgressEvent, VideoProbeInfo,
};
use optimizer::TranscodeOptions;

#[derive(Default)]
struct AllowedVideoPaths(Mutex<HashSet<PathBuf>>);

impl AllowedVideoPaths {
    fn insert(&self, path: &str) {
        let p = Path::new(path);
        let canonical = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        if let Ok(mut guard) = self.0.lock() {
            guard.insert(canonical);
        }
    }

    fn contains(&self, path: &str) -> Option<PathBuf> {
        let p = Path::new(path);
        let canonical = p.canonicalize().ok()?;
        let guard = self.0.lock().ok()?;
        if guard.contains(&canonical) {
            Some(canonical)
        } else {
            None
        }
    }
}

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

#[tauri::command]
fn probe_video(
    path: String,
    allowed: tauri::State<'_, AllowedVideoPaths>,
) -> Result<VideoProbeInfo, String> {
    let info = looper::probe_video(&path)?;
    allowed.insert(&path);
    Ok(info)
}

#[tauri::command]
async fn analyze_video_loop(
    path: String,
    min_loop_sec: Option<f64>,
) -> Result<LoopAnalysisResult, String> {
    tauri::async_runtime::spawn_blocking(move || looper::analyze_loop_points(&path, min_loop_sec, 5))
        .await
        .map_err(|e| format!("Ошибка фоновой задачи анализа: {}", e))?
}

#[tauri::command]
async fn process_video_loop(
    options: LoopProcessOptions,
    on_progress: tauri::ipc::Channel<LoopProgressEvent>,
    allowed: tauri::State<'_, AllowedVideoPaths>,
) -> Result<LoopProcessResult, String> {
    allowed.insert(&options.input_path);
    let result = tauri::async_runtime::spawn_blocking(move || {
        looper::process_loop_video(&options, |ev| {
            let _ = on_progress.send(ev);
        })
    })
    .await
    .map_err(|e| format!("Ошибка фоновой задачи кодирования: {}", e))??;

    allowed.insert(&result.output_path);
    Ok(result)
}

fn sanitize_file_name(raw: &str) -> String {
    let base = Path::new(raw)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("video.webm");
    let clean: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if clean.is_empty() {
        "video.webm".to_string()
    } else {
        clean
    }
}

#[tauri::command]
fn stage_uploaded_video(
    request: tauri::ipc::Request<'_>,
    allowed: tauri::State<'_, AllowedVideoPaths>,
) -> Result<VideoProbeInfo, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("Ожидались бинарные данные видеофайла".to_string());
    };
    if bytes.is_empty() {
        return Err("Передан пустой видеофайл".to_string());
    }

    let raw_name = request
        .headers()
        .get("x-file-name")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("video.webm");
    let safe_name = sanitize_file_name(raw_name);

    let dir = std::env::temp_dir().join("creator-2d-avatar-loops");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Не удалось создать временную папку: {}", e))?;

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let staged_path = dir.join(format!("{}_{}", ts, safe_name));
    std::fs::write(&staged_path, bytes)
        .map_err(|e| format!("Не удалось сохранить временный файл: {}", e))?;

    let path_str = staged_path.to_string_lossy().into_owned();
    allowed.insert(&path_str);
    looper::probe_video(&path_str)
}

#[tauri::command]
fn read_video_bytes(
    path: String,
    allowed: tauri::State<'_, AllowedVideoPaths>,
) -> Result<tauri::ipc::Response, String> {
    let canonical = allowed.contains(&path).ok_or_else(|| {
        format!(
            "Доступ к чтению файла '{}' запрещён: сначала откройте или экспортируйте его через приложение",
            path
        )
    })?;
    let bytes = std::fs::read(&canonical)
        .map_err(|e| format!("Не удалось прочитать файл '{}': {}", canonical.display(), e))?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
fn reveal_in_folder(path: String) -> Result<(), String> {
    tauri_plugin_opener::reveal_item_in_dir(&path)
        .map_err(|e| format!("Не удалось открыть файл в проводнике: {}", e))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AllowedVideoPaths::default())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_obs_url,
            preview_ffmpeg_args,
            probe_video,
            analyze_video_loop,
            process_video_loop,
            stage_uploaded_video,
            read_video_bytes,
            reveal_in_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
