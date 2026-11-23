use std::ffi::OsStr;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::looper::probe_video;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChromaKeyParams {
    pub color_hex: String, // e.g. "0x00FF00"
    pub similarity: f32,   // 0.0 - 1.0
    pub blend: f32,        // 0.0 - 1.0
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscodeOptions {
    pub input_path: String,
    pub output_path: String,
    pub resolution: Option<String>, // e.g. "1920x1080", "1280x720"
    pub chroma_key: Option<ChromaKeyParams>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizeProgressEvent {
    pub stage: String,
    pub percent: f32,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscodeResult {
    pub output_path: String,
    pub duration_sec: f64,
    pub frame_count: u64,
    pub file_size_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
    pub ffmpeg_args: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSheet {
    pub path: String,
    pub columns: u32,
    pub rows: u32,
    pub frame_count: u32,
    pub tile_width: u32,
    pub tile_height: u32,
    pub duration_sec: f64,
}

/// Снимает кадры через FFmpeg в один PNG-спрайтлист. WebKit/GStreamer
/// иногда декодирует WebM с альфой в серый шум, поэтому предпросмотр
/// строится по пикселям, которые отдаёт FFmpeg, а не <video>.
pub fn extract_preview_sheet(input_path: &str, max_frames: u32) -> Result<PreviewSheet, String> {
    let probe = probe_video(input_path)?;
    let max_frames = max_frames.clamp(8, 128);
    let fps = (max_frames as f64 / probe.duration_sec.max(0.5)).clamp(1.0, 8.0);
    let frame_count = ((probe.duration_sec * fps).floor() as u32).clamp(2, max_frames);
    let columns = 8u32;
    let rows = ((frame_count + columns - 1) / columns).max(1);
    let tile_w = 360u32;
    let tile_h = 202u32;

    let filter = format!(
        "fps={:.4},scale={}:{}:force_original_aspect_ratio=decrease,\
         pad={}:{}:(ow-iw)/2:(oh-ih)/2:color=black,format=rgba,\
         tile={}x{}:padding=0:margin=0",
        fps, tile_w, tile_h, tile_w, tile_h, columns, rows
    );

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let out = std::env::temp_dir().join(format!("creator-2d-avatar-preview-{}.png", stamp));
    let out_str = out.to_string_lossy().into_owned();

    let output = Command::new("ffmpeg")
        .args([
            "-y",
            "-v",
            "error",
            "-i",
            input_path,
            "-vf",
            &filter,
            "-frames:v",
            "1",
            "-pix_fmt",
            "rgba",
            "-update",
            "1",
            &out_str,
        ])
        .output()
        .map_err(|e| format!("Не удалось запустить ffmpeg для превью: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "FFmpeg не смог извлечь кадры превью: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(PreviewSheet {
        path: out_str,
        columns,
        rows,
        frame_count,
        tile_width: tile_w,
        tile_height: tile_h,
        duration_sec: probe.duration_sec,
    })
}

pub struct VideoOptimizer;

impl VideoOptimizer {
    pub fn build_ffmpeg_args(opts: &TranscodeOptions, output_path: &str) -> Vec<String> {
        let mut args: Vec<String> = vec!["-i".into(), opts.input_path.clone()];

        // Video filters: chroma key + scaling + alpha-capable pixel format.
        let mut filters: Vec<String> = Vec::new();
        if let Some(chroma) = &opts.chroma_key {
            // FFmpeg expects the colour as 0xRRGGBB (a leading '#' from the
            // colour picker must be stripped, otherwise colorkey fails).
            let color = chroma
                .color_hex
                .trim()
                .trim_start_matches('#')
                .trim_start_matches("0x")
                .trim_start_matches("0X");
            filters.push(format!(
                "colorkey=0x{}:{:.4}:{:.4}",
                color, chroma.similarity, chroma.blend
            ));
            // colorkey creates the alpha channel; keep it through the filters.
            filters.push("format=rgba".to_string());
        }

        if let Some(res) = &opts.resolution {
            filters.push(format!(
                "scale={}:force_original_aspect_ratio=decrease",
                res
            ));
        }
        // VP9 needs even dimensions for yuva420p.
        filters.push("scale=trunc(iw/2)*2:trunc(ih/2)*2".to_string());
        filters.push("format=yuva420p".to_string());

        args.push("-vf".into());
        args.push(filters.join(","));

        // WebM VP9 settings with true alpha channel.
        args.extend(
            [
                "-c:v",
                "libvpx-vp9",
                "-pix_fmt",
                "yuva420p",
                "-b:v",
                "2M",
                "-auto-alt-ref",
                "0",
                "-an", // remove audio track for avatar loops
            ]
            .iter()
            .map(|s| s.to_string()),
        );
        args.push(output_path.to_string());
        args
    }

    /// Builds the FFmpeg command line arguments for exporting transparent WebM VP9.
    pub fn build_ffmpeg_command(opts: &TranscodeOptions) -> Command {
        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y");
        cmd.args(
            Self::build_ffmpeg_args(opts, &opts.output_path)
                .iter()
                .map(|s| s.as_str()),
        );
        cmd
    }
}

fn resolve_output_path(opts: &TranscodeOptions) -> Result<PathBuf, String> {
    if !opts.output_path.trim().is_empty() {
        return Ok(PathBuf::from(opts.output_path.trim()));
    }
    let input = Path::new(&opts.input_path);
    let parent = input.parent().unwrap_or_else(|| Path::new("."));
    let stem = input
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("avatar");
    Ok(parent.join(format!("{}_transparent.webm", stem)))
}

/// Runs the real FFmpeg transcode with streaming progress events.
pub fn optimize_video<F>(
    opts: &TranscodeOptions,
    mut on_progress: F,
) -> Result<TranscodeResult, String>
where
    F: FnMut(OptimizeProgressEvent),
{
    on_progress(OptimizeProgressEvent {
        stage: "probe".to_string(),
        percent: 3.0,
        message: "Анализ исходного видео...".to_string(),
    });

    let input_probe = probe_video(&opts.input_path)?;
    let output_path = resolve_output_path(opts)?;
    let output_str = output_path.to_string_lossy().into_owned();
    let args = VideoOptimizer::build_ffmpeg_args(opts, &output_str);

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Не удалось создать папку для результата: {}", e))?;
        }
    }

    on_progress(OptimizeProgressEvent {
        stage: "encode".to_string(),
        percent: 12.0,
        message: "Кодирование WebM VP9 с альфой...".to_string(),
    });

    let mut run_args = vec![
        "-y".to_string(),
        "-progress".to_string(),
        "pipe:1".to_string(),
        "-nostats".to_string(),
        "-v".to_string(),
        "error".to_string(),
    ];
    run_args.extend(args.clone());

    let mut child = Command::new("ffmpeg")
        .args(&run_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Не удалось запустить ffmpeg: {}", e))?;

    let stderr_handle = child.stderr.take().map(|stderr| {
        thread::spawn(move || {
            let mut buf = String::new();
            let mut reader = BufReader::new(stderr);
            let _ = std::io::Read::read_to_string(&mut reader, &mut buf);
            buf
        })
    });

    let expected_us = (input_probe.duration_sec * 1_000_000.0).max(1.0);
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(value) = line.strip_prefix("out_time_us=") {
                if let Ok(us) = value.trim().parse::<f64>() {
                    let frac = (us / expected_us).clamp(0.0, 1.0);
                    on_progress(OptimizeProgressEvent {
                        stage: "encode".to_string(),
                        percent: 12.0 + frac as f32 * 80.0,
                        message: format!("Кодирование видео ({:.0}%)...", frac * 100.0),
                    });
                }
            }
        }
    }

    let status = child
        .wait()
        .map_err(|e| format!("Ошибка ожидания завершения ffmpeg: {}", e))?;
    let stderr_text = stderr_handle
        .and_then(|h| h.join().ok())
        .unwrap_or_default();
    if !status.success() {
        return Err(format!(
            "FFmpeg завершился с кодом {:?}: {}",
            status.code(),
            stderr_text.trim()
        ));
    }

    on_progress(OptimizeProgressEvent {
        stage: "verify".to_string(),
        percent: 95.0,
        message: "Проверка результата...".to_string(),
    });

    let out_probe = probe_video(&output_str)?;
    on_progress(OptimizeProgressEvent {
        stage: "done".to_string(),
        percent: 100.0,
        message: "Конвертация завершена!".to_string(),
    });

    Ok(TranscodeResult {
        output_path: output_str,
        duration_sec: out_probe.duration_sec,
        frame_count: out_probe.frame_count,
        file_size_bytes: out_probe.file_size_bytes,
        width: out_probe.width,
        height: out_probe.height,
        has_alpha: out_probe.has_alpha,
        ffmpeg_args: args,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(chroma: Option<ChromaKeyParams>) -> TranscodeOptions {
        TranscodeOptions {
            input_path: "/tmp/input.mov".into(),
            output_path: "/tmp/output.webm".into(),
            resolution: Some("1280x720".into()),
            chroma_key: chroma,
        }
    }

    #[test]
    fn chroma_key_is_stripped_and_alpha_is_preserved() {
        let args = VideoOptimizer::build_ffmpeg_args(
            &opts(Some(ChromaKeyParams {
                color_hex: "#00FF00".into(),
                similarity: 0.3,
                blend: 0.1,
            })),
            "/tmp/output.webm",
        );
        let joined = args.join(" ");
        assert!(joined.contains("colorkey=0x00FF00:0.3000:0.1000"));
        assert!(joined.contains("format=rgba"));
        assert!(joined.contains("-pix_fmt yuva420p"));
    }

    #[test]
    fn ipc_payloads_use_camel_case() {
        let opts_json = serde_json::to_value(&opts(Some(ChromaKeyParams {
            color_hex: "#00FF00".into(),
            similarity: 0.3,
            blend: 0.1,
        })))
        .unwrap();
        assert!(opts_json.get("inputPath").is_some());
        assert!(opts_json.get("chromaKey").is_some());

        let result_json = serde_json::to_value(&TranscodeResult {
            output_path: "/tmp/out.webm".into(),
            duration_sec: 1.0,
            frame_count: 24,
            file_size_bytes: 1024,
            width: 320,
            height: 240,
            has_alpha: true,
            ffmpeg_args: vec![],
        })
        .unwrap();
        assert!(result_json.get("outputPath").is_some());
        assert!(result_json.get("fileSizeBytes").is_some());
        assert!(result_json.get("hasAlpha").is_some());
    }

    #[test]
    fn output_path_defaults_to_source_stem() {
        let path = resolve_output_path(&TranscodeOptions {
            input_path: "/tmp/clips/avatar.mov".into(),
            output_path: String::new(),
            resolution: None,
            chroma_key: None,
        })
        .unwrap();
        assert_eq!(path, PathBuf::from("/tmp/clips/avatar_transparent.webm"));
    }
}
