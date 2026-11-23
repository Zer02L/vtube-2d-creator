use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::optimizer::ChromaKeyParams;

const ANALYSIS_DIM: usize = 48;
const ANALYSIS_FRAME_BYTES: usize = ANALYSIS_DIM * ANALYSIS_DIM * 4;
const SEAM_DIM: usize = 128;
const SEAM_FRAME_BYTES: usize = SEAM_DIM * SEAM_DIM * 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopMode {
    Crossfade,
    PingPong,
    Morph,
    SmartCut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Webm,
    Mp4,
    Gif,
    Mov,
}

impl OutputFormat {
    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::Webm => "webm",
            OutputFormat::Mp4 => "mp4",
            OutputFormat::Gif => "gif",
            OutputFormat::Mov => "mov",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoProbeInfo {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_sec: f64,
    pub frame_count: u64,
    pub codec_name: String,
    pub pix_fmt: String,
    pub has_alpha: bool,
    pub file_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopCandidate {
    pub start_sec: f64,
    pub end_sec: f64,
    pub duration_sec: f64,
    pub similarity: f64,
    pub pose_similarity: f64,
    pub motion_similarity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopAnalysisResult {
    pub probe: VideoProbeInfo,
    pub original_seam_similarity: f64,
    pub sampled_fps: f64,
    pub sampled_frames: usize,
    pub candidates: Vec<LoopCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopProcessOptions {
    pub input_path: String,
    pub output_path: Option<String>,
    pub mode: LoopMode,
    pub start_sec: Option<f64>,
    pub end_sec: Option<f64>,
    pub crossfade_sec: f64,
    pub bridge_sec: f64,
    pub exact_end_frame: bool,
    pub format: OutputFormat,
    pub resolution: Option<String>,
    pub target_fps: Option<f64>,
    pub chroma_key: Option<ChromaKeyParams>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopProgressEvent {
    pub stage: String,
    pub percent: f32,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopProcessResult {
    pub output_path: String,
    pub duration_sec: f64,
    pub frame_count: u64,
    pub file_size_bytes: u64,
    pub original_seam_similarity: f64,
    pub output_seam_similarity: f64,
    pub ffmpeg_args: Vec<String>,
}

/// Parses FFmpeg rational frame rates such as "30/1", "30000/1001", or "29.97".
pub fn parse_rational_fps(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some((num_s, den_s)) = trimmed.split_once('/') {
        let num: f64 = num_s.trim().parse().ok()?;
        let den: f64 = den_s.trim().parse().ok()?;
        if den <= 0.0 || num <= 0.0 || !num.is_finite() || !den.is_finite() {
            return None;
        }
        let fps = num / den;
        if fps.is_finite() && fps > 0.0 {
            Some(fps)
        } else {
            None
        }
    } else {
        let fps: f64 = trimmed.parse().ok()?;
        if fps.is_finite() && fps > 0.0 {
            Some(fps)
        } else {
            None
        }
    }
}

fn pix_fmt_has_alpha(pix_fmt: &str) -> bool {
    let lower = pix_fmt.to_ascii_lowercase();
    lower.contains("rgba")
        || lower.contains("bgra")
        || lower.contains("argb")
        || lower.contains("abgr")
        || lower.contains("yuva")
        || lower.contains("ya")
        || lower.contains("gbrap")
        || lower == "pal8"
}

/// Probes video stream metadata via ffprobe.
pub fn probe_video(path: &str) -> Result<VideoProbeInfo, String> {
    if !Path::new(path).exists() {
        return Err(format!("Входной файл не найден: {}", path));
    }

    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,pix_fmt,r_frame_rate,avg_frame_rate,nb_frames,codec_name:stream_tags=alpha_mode",
            "-show_entries",
            "format=duration,size",
            "-of",
            "json",
            path,
        ])
        .output()
        .map_err(|e| {
            format!(
                "Не удалось запустить ffprobe (убедитесь, что FFmpeg установлен и доступен в PATH): {}",
                e
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Ошибка ffprobe для '{}': {}", path, stderr.trim()));
    }

    let json: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Не удалось разобрать JSON от ffprobe: {}", e))?;

    let stream = json["streams"]
        .as_array()
        .and_then(|arr| arr.first())
        .ok_or_else(|| format!("В файле '{}' не найден видеопоток", path))?;

    let width = stream["width"]
        .as_u64()
        .ok_or_else(|| "ffprobe не вернул ширину видео".to_string())? as u32;
    let height = stream["height"]
        .as_u64()
        .ok_or_else(|| "ffprobe не вернул высоту видео".to_string())? as u32;

    let codec_name = stream["codec_name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let pix_fmt = stream["pix_fmt"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    let alpha_mode = stream["tags"]["alpha_mode"].as_str().unwrap_or("0");
    let has_alpha = pix_fmt_has_alpha(&pix_fmt) || alpha_mode == "1";

    let fps = stream["r_frame_rate"]
        .as_str()
        .and_then(parse_rational_fps)
        .or_else(|| stream["avg_frame_rate"].as_str().and_then(parse_rational_fps))
        .unwrap_or(30.0);

    let nb_frames_stream = stream["nb_frames"]
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| stream["nb_frames"].as_u64());

    let format_duration = json["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .or_else(|| json["format"]["duration"].as_f64())
        .filter(|d| d.is_finite() && *d > 0.0);

    let (duration_sec, frame_count) = match (format_duration, nb_frames_stream) {
        (Some(dur), Some(cnt)) => (dur, cnt),
        (Some(dur), None) => (dur, (dur * fps).round().max(1.0) as u64),
        (None, Some(cnt)) => ((cnt as f64) / fps, cnt),
        (None, None) => {
            // Fallback for containers like animated GIF without header frame counts/duration.
            let counted = count_frames_fallback(path).unwrap_or(1);
            ((counted as f64) / fps, counted)
        }
    };

    let file_size_bytes = json["format"]["size"]
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| json["format"]["size"].as_u64())
        .unwrap_or_else(|| std::fs::metadata(path).map(|m| m.len()).unwrap_or(0));

    Ok(VideoProbeInfo {
        path: path.to_string(),
        width,
        height,
        fps,
        duration_sec,
        frame_count,
        codec_name,
        pix_fmt,
        has_alpha,
        file_size_bytes,
    })
}

fn count_frames_fallback(path: &str) -> Option<u64> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-count_frames",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=nb_read_frames",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            path,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse::<u64>().ok()
}

/// Returns explicit decoder flags required so FFmpeg preserves alpha in VP8/VP9 WebM inputs.
pub fn input_decoder_args(probe: &VideoProbeInfo) -> Vec<String> {
    if probe.has_alpha {
        match probe.codec_name.as_str() {
            "vp9" => return vec!["-c:v".to_string(), "libvpx-vp9".to_string()],
            "vp8" => return vec!["-c:v".to_string(), "libvpx".to_string()],
            _ => {}
        }
    }
    Vec::new()
}

/// Computes normalized frame similarity in percent [0.0, 100.0] from equal-sized RGBA buffers.
pub fn frame_similarity_percent(a: &[u8], b: &[u8]) -> f64 {
    let dist = mean_abs_diff_norm(a, b);
    (100.0 * (1.0 - dist)).clamp(0.0, 100.0)
}

fn mean_abs_diff_norm(a: &[u8], b: &[u8]) -> f64 {
    if a.is_empty() || a.len() != b.len() {
        return 1.0;
    }
    let sum: u64 = a
        .iter()
        .zip(b.iter())
        .map(|(&x, &y)| x.abs_diff(y) as u64)
        .sum();
    (sum as f64) / (a.len() as f64 * 255.0)
}

fn motion_diff_norm(a0: &[u8], a1: &[u8], b_prev: &[u8], b_curr: &[u8]) -> f64 {
    let len = a0.len();
    if len == 0 || a1.len() != len || b_prev.len() != len || b_curr.len() != len {
        return 1.0;
    }
    let sum: u64 = (0..len)
        .map(|i| {
            let da = a1[i] as i32 - a0[i] as i32;
            let db = b_curr[i] as i32 - b_prev[i] as i32;
            (da - db).unsigned_abs() as u64
        })
        .sum();
    ((sum as f64) / (len as f64 * 255.0)).clamp(0.0, 1.0)
}

/// Finds the best candidate (start, end) loop intervals from sampled RGBA frames.
pub fn find_best_candidates(
    frames: &[Vec<u8>],
    sample_fps: f64,
    min_loop_sec: f64,
    max_candidates: usize,
) -> Vec<LoopCandidate> {
    if frames.len() < 3 || sample_fps <= 0.0 || max_candidates == 0 {
        return Vec::new();
    }

    let min_frames = ((min_loop_sec * sample_fps).round() as usize)
        .max(2)
        .min(frames.len() - 1);

    struct RawCandidate {
        start_sec: f64,
        end_sec: f64,
        combined: f64,
        pose_dist: f64,
        motion_dist: f64,
    }

    let mut raw = Vec::new();
    for i in 0..frames.len() {
        let j_start = i + min_frames;
        if j_start >= frames.len() {
            break;
        }
        for j in j_start..frames.len() {
            let pose_dist = mean_abs_diff_norm(&frames[i], &frames[j]);
            let motion_dist = if i + 1 < frames.len() && j >= 1 {
                motion_diff_norm(&frames[i], &frames[i + 1], &frames[j - 1], &frames[j])
            } else {
                pose_dist
            };
            let combined = 0.78 * pose_dist + 0.22 * motion_dist;
            raw.push(RawCandidate {
                start_sec: (i as f64) / sample_fps,
                end_sec: (j as f64) / sample_fps,
                combined,
                pose_dist,
                motion_dist,
            });
        }
    }

    raw.sort_by(|a, b| {
        a.combined
            .partial_cmp(&b.combined)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut kept: Vec<LoopCandidate> = Vec::with_capacity(max_candidates);
    for cand in raw {
        let overlaps = kept.iter().any(|existing| {
            (cand.start_sec - existing.start_sec).abs() < 0.35
                && (cand.end_sec - existing.end_sec).abs() < 0.35
        });
        if overlaps {
            continue;
        }
        let duration_sec = (cand.end_sec - cand.start_sec).max(0.0);
        kept.push(LoopCandidate {
            start_sec: (cand.start_sec * 1000.0).round() / 1000.0,
            end_sec: (cand.end_sec * 1000.0).round() / 1000.0,
            duration_sec: (duration_sec * 1000.0).round() / 1000.0,
            similarity: ((1.0 - cand.combined) * 100.0).clamp(0.0, 100.0),
            pose_similarity: ((1.0 - cand.pose_dist) * 100.0).clamp(0.0, 100.0),
            motion_similarity: ((1.0 - cand.motion_dist) * 100.0).clamp(0.0, 100.0),
        });
        if kept.len() >= max_candidates {
            break;
        }
    }

    kept
}

/// Decodes low-resolution frames from the input video and ranks optimal seamless loop segments.
pub fn analyze_loop_points(
    path: &str,
    min_loop_sec: Option<f64>,
    max_candidates: usize,
) -> Result<LoopAnalysisResult, String> {
    let probe = probe_video(path)?;
    let sample_fps = (150.0 / probe.duration_sec.max(0.2)).clamp(4.0, probe.fps.min(15.0));

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-v").arg("error");
    cmd.args(input_decoder_args(&probe));
    cmd.arg("-i").arg(path);
    cmd.arg("-vf").arg(format!(
        "fps={:.6},scale={}:{}:flags=bilinear",
        sample_fps, ANALYSIS_DIM, ANALYSIS_DIM
    ));
    cmd.args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"]);

    let out = cmd
        .output()
        .map_err(|e| format!("Не удалось запустить ffmpeg для анализа: {}", e))?;

    if !out.status.success() {
        return Err(format!(
            "Ошибка ffmpeg при анализе '{}': {}",
            path,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    let frames: Vec<Vec<u8>> = out
        .stdout
        .chunks_exact(ANALYSIS_FRAME_BYTES)
        .map(|chunk| chunk.to_vec())
        .collect();

    if frames.len() < 2 {
        return Err("Видео слишком короткое: для анализа нужно минимум 2 кадра".to_string());
    }

    let original_seam_similarity = frame_similarity_percent(&frames[0], &frames[frames.len() - 1]);
    let min_sec = min_loop_sec
        .unwrap_or_else(|| (probe.duration_sec * 0.35).clamp(0.4, 1.2))
        .min((probe.duration_sec * 0.85).max(0.2));

    let candidates = find_best_candidates(&frames, sample_fps, min_sec, max_candidates);

    Ok(LoopAnalysisResult {
        probe,
        original_seam_similarity,
        sampled_fps: sample_fps,
        sampled_frames: frames.len(),
        candidates,
    })
}

fn extract_raw_frames(
    path: &str,
    probe: &VideoProbeInfo,
    extra_input_args: &[&str],
    extra_output_args: &[&str],
) -> Result<Vec<u8>, String> {
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-v").arg("error");
    cmd.args(extra_input_args);
    cmd.args(input_decoder_args(probe));
    cmd.arg("-i").arg(path);
    cmd.args(extra_output_args);
    cmd.arg("-vf")
        .arg(format!("scale={}:{}:flags=bilinear", SEAM_DIM, SEAM_DIM));
    cmd.args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"]);

    let out = cmd
        .output()
        .map_err(|e| format!("Не удалось запустить ffmpeg для проверки шва: {}", e))?;

    if !out.status.success() {
        return Err(format!(
            "Ошибка ffmpeg при чтении кадра '{}': {}",
            path,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    Ok(out.stdout)
}

/// Measures first-frame vs last-frame similarity (0-100%) on the actual encoded video file.
pub fn measure_seam_similarity(path: &str, probe: &VideoProbeInfo) -> Result<f64, String> {
    let first_raw = extract_raw_frames(path, probe, &[], &["-frames:v", "1"])?;
    if first_raw.len() < SEAM_FRAME_BYTES {
        return Err(format!("Не удалось декодировать первый кадр '{}'", path));
    }
    let first = &first_raw[..SEAM_FRAME_BYTES];

    let mut last_raw = if probe.duration_sec > 2.0 {
        extract_raw_frames(path, probe, &["-sseof", "-1.5"], &[]).unwrap_or_default()
    } else {
        Vec::new()
    };

    if last_raw.len() < SEAM_FRAME_BYTES {
        last_raw = extract_raw_frames(path, probe, &[], &[])?;
    }

    let complete_frames = last_raw.len() / SEAM_FRAME_BYTES;
    if complete_frames == 0 {
        return Err(format!("Не удалось декодировать последний кадр '{}'", path));
    }

    let last_offset = (complete_frames - 1) * SEAM_FRAME_BYTES;
    let last = &last_raw[last_offset..last_offset + SEAM_FRAME_BYTES];
    Ok(frame_similarity_percent(first, last))
}

/// Resolves the destination file path from explicit user input or input path + target format.
pub fn derive_output_path(opts: &LoopProcessOptions) -> String {
    if let Some(custom) = &opts.output_path {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let input = Path::new(&opts.input_path);
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("avatar");
    let dir = input.parent().filter(|p| !p.as_os_str().is_empty());
    let file_name = format!("{}_loop.{}", stem, opts.format.extension());
    let out: PathBuf = match dir {
        Some(parent) => parent.join(file_name),
        None => PathBuf::from(file_name),
    };
    out.to_string_lossy().into_owned()
}

fn normalize_resolution(res: &str) -> Option<String> {
    let trimmed = res.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("source") {
        return None;
    }
    Some(trimmed.replace('x', ":").replace('X', ":"))
}

/// Constructs FFmpeg CLI arguments, resolved output path, and expected output duration.
pub fn build_loop_ffmpeg_args(
    opts: &LoopProcessOptions,
    probe: &VideoProbeInfo,
) -> Result<(Vec<String>, String, f64), String> {
    let start = opts.start_sec.unwrap_or(0.0).max(0.0);
    let raw_end = opts.end_sec.unwrap_or(probe.duration_sec);
    let end = raw_end.min(probe.duration_sec);
    let seg_dur = end - start;
    if seg_dur < 0.15 {
        return Err(format!(
            "Выбранный отрезок слишком короткий ({:.3}с). Минимум 0.15с.",
            seg_dur
        ));
    }

    let out_fps = opts.target_fps.unwrap_or(probe.fps).clamp(1.0, 120.0);
    let wants_alpha = matches!(
        opts.format,
        OutputFormat::Webm | OutputFormat::Gif | OutputFormat::Mov
    ) && (probe.has_alpha || opts.chroma_key.is_some());

    // Stage 1: [0:v] -> [pre]
    let mut pre_filters = vec![
        format!("trim=start={:.6}:end={:.6}", start, end),
        "setpts=PTS-STARTPTS".to_string(),
        format!("fps={:.6}", out_fps),
    ];

    if let Some(ck) = &opts.chroma_key {
        let hex = ck
            .color_hex
            .trim()
            .trim_start_matches('#')
            .trim_start_matches("0x")
            .trim_start_matches("0X");
        pre_filters.push(format!("colorkey=0x{}:{:.4}:{:.4}", hex, ck.similarity, ck.blend));
    }

    if let Some(res_str) = opts.resolution.as_deref().and_then(normalize_resolution) {
        pre_filters.push(format!(
            "scale={}:force_original_aspect_ratio=decrease",
            res_str
        ));
    }

    if matches!(opts.format, OutputFormat::Webm | OutputFormat::Mp4) {
        pre_filters.push("scale=trunc(iw/2)*2:trunc(ih/2)*2".to_string());
    }

    pre_filters.push(if wants_alpha {
        "format=rgba".to_string()
    } else {
        "format=rgb24".to_string()
    });

    let mut graph_parts = vec![format!("[0:v]{}[pre]", pre_filters.join(","))];

    // Stage 2: [pre] -> [looped]
    let mut handled_exact_end = false;
    let mut expected_dur: f64;

    match opts.mode {
        LoopMode::Crossfade => {
            let fade = opts
                .crossfade_sec
                .min(seg_dur * 0.45)
                .max(1.0 / out_fps);
            let offset = (seg_dur - 2.0 * fade).max(0.0);
            graph_parts.push(format!(
                "[pre]split=2[cf_a][cf_b];\
                 [cf_a]trim=start={fade:.6}:end={seg_dur:.6},setpts=PTS-STARTPTS[cf_main];\
                 [cf_b]trim=start=0:end={fade:.6},setpts=PTS-STARTPTS[cf_head];\
                 [cf_main][cf_head]xfade=transition=fade:duration={fade:.6}:offset={offset:.6}[looped]"
            ));
            expected_dur = (seg_dur - fade).max(1.0 / out_fps);
        }
        LoopMode::SmartCut => {
            if opts.crossfade_sec > 0.02 {
                let fade = opts
                    .crossfade_sec
                    .min(seg_dur * 0.45)
                    .max(1.0 / out_fps);
                let offset = (seg_dur - 2.0 * fade).max(0.0);
                graph_parts.push(format!(
                    "[pre]split=2[cf_a][cf_b];\
                     [cf_a]trim=start={fade:.6}:end={seg_dur:.6},setpts=PTS-STARTPTS[cf_main];\
                     [cf_b]trim=start=0:end={fade:.6},setpts=PTS-STARTPTS[cf_head];\
                     [cf_main][cf_head]xfade=transition=fade:duration={fade:.6}:offset={offset:.6}[looped]"
                ));
                expected_dur = (seg_dur - fade).max(1.0 / out_fps);
            } else {
                graph_parts.push("[pre]null[looped]".to_string());
                expected_dur = seg_dur;
            }
        }
        LoopMode::PingPong => {
            let n_frames = (seg_dur * out_fps).round().max(2.0) as u64;
            if n_frames <= 2 {
                graph_parts.push("[pre]null[looped]".to_string());
                expected_dur = seg_dur;
            } else {
                let rev_end = if opts.exact_end_frame {
                    n_frames
                } else {
                    n_frames - 1
                };
                handled_exact_end = true;
                graph_parts.push(format!(
                    "[pre]trim=start_frame=0:end_frame={n_frames},setpts=PTS-STARTPTS,split=2[pp_fwd][pp_rev_in];\
                     [pp_rev_in]reverse,trim=start_frame=1:end_frame={rev_end},setpts=PTS-STARTPTS[pp_rev];\
                     [pp_fwd][pp_rev]concat=n=2:v=1:a=0[looped]"
                ));
                expected_dur = (n_frames + rev_end - 1) as f64 / out_fps;
            }
        }
        LoopMode::Morph => {
            let bridge_sec = opts.bridge_sec.clamp(0.1, 2.0);
            let bridge_frames = (bridge_sec * out_fps).round().max(2.0) as u64;
            let bridge_ms = (bridge_sec * 1000.0).round().max(50.0) as u64;
            let bridge_trim_end = if opts.exact_end_frame {
                bridge_frames + 1
            } else {
                bridge_frames
            };
            handled_exact_end = true;

            // Feed 3 frames [last, first, first] so minterpolate flushes the full [last -> first] span.
            graph_parts.push(format!(
                "[pre]split=4[m_orig][m_last_in][m_f1_in][m_f2_in];\
                 [m_last_in]reverse,trim=start_frame=0:end_frame=1,setpts=PTS-STARTPTS[m_last];\
                 [m_f1_in]trim=start_frame=0:end_frame=1,setpts=PTS-STARTPTS[m_f1];\
                 [m_f2_in]trim=start_frame=0:end_frame=1,setpts=PTS-STARTPTS[m_f2];\
                 [m_last][m_f1][m_f2]concat=n=3:v=1:a=0,settb=1/1000,setpts=N*{bridge_ms}[m_pair]"
            ));

            if wants_alpha {
                graph_parts.push(format!(
                    "[m_pair]split=2[mp_rgb][mp_a];\
                     [mp_rgb]format=rgb24,minterpolate=fps={out_fps:.6}:mi_mode=mci:mc_mode=aobmc:me_mode=bidir:vsbmc=1:scd=none,trim=start_frame=1:end_frame={bridge_trim_end},setpts=PTS-STARTPTS[mb_rgb];\
                     [mp_a]alphaextract,minterpolate=fps={out_fps:.6}:mi_mode=mci:mc_mode=aobmc:me_mode=bidir:vsbmc=1:scd=none,trim=start_frame=1:end_frame={bridge_trim_end},setpts=PTS-STARTPTS[mb_a];\
                     [mb_rgb][mb_a]alphamerge,format=rgba,fps={out_fps:.6}[m_bridge]"
                ));
            } else {
                graph_parts.push(format!(
                    "[m_pair]format=rgb24,minterpolate=fps={out_fps:.6}:mi_mode=mci:mc_mode=aobmc:me_mode=bidir:vsbmc=1:scd=none,trim=start_frame=1:end_frame={bridge_trim_end},setpts=PTS-STARTPTS,fps={out_fps:.6}[m_bridge]"
                ));
            }

            graph_parts.push("[m_orig][m_bridge]concat=n=2:v=1:a=0[looped]".to_string());
            expected_dur = seg_dur + (bridge_trim_end - 1) as f64 / out_fps;
        }
    }

    // Stage 3: [looped] -> [final]
    if opts.exact_end_frame && !handled_exact_end {
        graph_parts.push(
            "[looped]split=2[ex_body][ex_f0_in];\
             [ex_f0_in]trim=start_frame=0:end_frame=1,setpts=PTS-STARTPTS[ex_f0];\
             [ex_body][ex_f0]concat=n=2:v=1:a=0[final]"
                .to_string(),
        );
        expected_dur += 1.0 / out_fps;
    } else {
        graph_parts.push("[looped]null[final]".to_string());
    }

    // Stage 4: [final] -> [out] + encoder flags
    let mut codec_args: Vec<String> = Vec::new();
    match opts.format {
        OutputFormat::Webm => {
            let pix = if wants_alpha { "yuva420p" } else { "yuv420p" };
            graph_parts.push(format!("[final]format={}[out]", pix));
            codec_args.extend(
                [
                    "-map",
                    "[out]",
                    "-c:v",
                    "libvpx-vp9",
                    "-pix_fmt",
                    pix,
                    "-b:v",
                    "0",
                    "-crf",
                    "24",
                    "-auto-alt-ref",
                    "0",
                    "-deadline",
                    "good",
                    "-cpu-used",
                    "2",
                    "-an",
                ]
                .into_iter()
                .map(String::from),
            );
        }
        OutputFormat::Mp4 => {
            graph_parts.push("[final]format=yuv420p[out]".to_string());
            codec_args.extend(
                [
                    "-map",
                    "[out]",
                    "-c:v",
                    "libx264",
                    "-pix_fmt",
                    "yuv420p",
                    "-crf",
                    "18",
                    "-preset",
                    "medium",
                    "-movflags",
                    "+faststart",
                    "-an",
                ]
                .into_iter()
                .map(String::from),
            );
        }
        OutputFormat::Gif => {
            graph_parts.push(
                "[final]split=2[g1][g2];\
                 [g1]palettegen=stats_mode=diff:reserve_transparent=on[pal];\
                 [g2][pal]paletteuse=dither=bayer:bayer_scale=3[out]"
                    .to_string(),
            );
            codec_args.extend(["-map", "[out]", "-loop", "0", "-an"].into_iter().map(String::from));
        }
        OutputFormat::Mov => {
            graph_parts.push("[final]format=yuva444p10le[out]".to_string());
            codec_args.extend(
                [
                    "-map",
                    "[out]",
                    "-c:v",
                    "prores_ks",
                    "-profile:v",
                    "4444",
                    "-pix_fmt",
                    "yuva444p10le",
                    "-an",
                ]
                .into_iter()
                .map(String::from),
            );
        }
    }

    let output_path = derive_output_path(opts);
    let mut args = vec!["-y".to_string()];
    args.extend(input_decoder_args(probe));
    args.extend([
        "-i".to_string(),
        opts.input_path.clone(),
        "-filter_complex".to_string(),
        graph_parts.join(";"),
    ]);
    args.extend(codec_args);
    args.push(output_path.clone());

    Ok((args, output_path, expected_dur))
}

/// Processes a video into a seamless loop, streaming progress events and verifying output seam similarity.
pub fn process_loop_video<F>(
    opts: &LoopProcessOptions,
    mut on_progress: F,
) -> Result<LoopProcessResult, String>
where
    F: FnMut(LoopProgressEvent),
{
    on_progress(LoopProgressEvent {
        stage: "probe".to_string(),
        percent: 4.0,
        message: "Анализ параметров исходного видео...".to_string(),
    });

    let probe = probe_video(&opts.input_path)?;
    let original_seam_similarity = measure_seam_similarity(&opts.input_path, &probe)?;

    on_progress(LoopProgressEvent {
        stage: "prepare".to_string(),
        percent: 10.0,
        message: "Построение бесшовного графа FFmpeg...".to_string(),
    });

    let (clean_args, output_path, expected_dur) = build_loop_ffmpeg_args(opts, &probe)?;

    if let Some(parent) = Path::new(&output_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Не удалось создать папку для результата: {}", e))?;
        }
    }

    let mut run_args = vec![
        "-y".to_string(),
        "-progress".to_string(),
        "pipe:1".to_string(),
        "-nostats".to_string(),
        "-v".to_string(),
        "error".to_string(),
    ];
    run_args.extend_from_slice(&clean_args[1..]);

    on_progress(LoopProgressEvent {
        stage: "encode".to_string(),
        percent: 15.0,
        message: "Кодирование бесшовного цикла...".to_string(),
    });

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

    let expected_us = (expected_dur * 1_000_000.0).max(1.0);
    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if let Some(val_str) = line.strip_prefix("out_time_us=") {
                if let Ok(us) = val_str.trim().parse::<f64>() {
                    let frac = (us / expected_us).clamp(0.0, 1.0);
                    let pct = (15.0 + frac * 77.0) as f32;
                    on_progress(LoopProgressEvent {
                        stage: "encode".to_string(),
                        percent: pct,
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

    on_progress(LoopProgressEvent {
        stage: "verify".to_string(),
        percent: 95.0,
        message: "Проверка совпадения первого и последнего кадра...".to_string(),
    });

    let out_probe = probe_video(&output_path)?;
    let output_seam_similarity = measure_seam_similarity(&output_path, &out_probe)?;

    on_progress(LoopProgressEvent {
        stage: "done".to_string(),
        percent: 100.0,
        message: "Бесшовный цикл готов!".to_string(),
    });

    Ok(LoopProcessResult {
        output_path,
        duration_sec: out_probe.duration_sec,
        frame_count: out_probe.frame_count,
        file_size_bytes: out_probe.file_size_bytes,
        original_seam_similarity,
        output_seam_similarity,
        ffmpeg_args: clean_args,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_probe(has_alpha: bool, codec: &str) -> VideoProbeInfo {
        VideoProbeInfo {
            path: "/tmp/input.webm".to_string(),
            width: 1280,
            height: 720,
            fps: 30.0,
            duration_sec: 3.0,
            frame_count: 90,
            codec_name: codec.to_string(),
            pix_fmt: if has_alpha {
                "yuva420p".to_string()
            } else {
                "yuv420p".to_string()
            },
            has_alpha,
            file_size_bytes: 1024,
        }
    }

    fn base_opts(mode: LoopMode, format: OutputFormat) -> LoopProcessOptions {
        LoopProcessOptions {
            input_path: "/tmp/avatar_idle.webm".to_string(),
            output_path: None,
            mode,
            start_sec: None,
            end_sec: None,
            crossfade_sec: 0.5,
            bridge_sec: 0.4,
            exact_end_frame: false,
            format,
            resolution: Some("1280x720".to_string()),
            target_fps: None,
            chroma_key: None,
        }
    }

    #[test]
    fn parses_rational_and_decimal_fps() {
        assert_eq!(parse_rational_fps("30/1"), Some(30.0));
        let ntsc = parse_rational_fps("30000/1001").unwrap();
        assert!((ntsc - 29.97002997).abs() < 1e-4);
        assert_eq!(parse_rational_fps("24"), Some(24.0));
        assert_eq!(parse_rational_fps("0/0"), None);
        assert_eq!(parse_rational_fps("-5/1"), None);
        assert_eq!(parse_rational_fps(""), None);
    }

    #[test]
    fn derives_output_path_per_format() {
        let mut opts = base_opts(LoopMode::Crossfade, OutputFormat::Webm);
        assert_eq!(derive_output_path(&opts), "/tmp/avatar_idle_loop.webm");
        opts.format = OutputFormat::Gif;
        assert_eq!(derive_output_path(&opts), "/tmp/avatar_idle_loop.gif");
        opts.output_path = Some("/tmp/custom_out.gif".to_string());
        assert_eq!(derive_output_path(&opts), "/tmp/custom_out.gif");
    }

    #[test]
    fn finds_repeating_pose_and_velocity_candidates() {
        // Synthetic 10fps, 3s (30 frames) signal oscillating linearly in [0..200].
        // Frame 4 (value 80, moving up) matches frame 24 (value 80, moving up),
        // whereas frame 12 (value 80, moving down) has reversed velocity.
        let vals: Vec<u8> = (0..30)
            .map(|i| {
                let phase = i % 20;
                if phase <= 10 {
                    (phase * 20) as u8
                } else {
                    ((20 - phase) * 20) as u8
                }
            })
            .collect();
        let frames: Vec<Vec<u8>> = vals.into_iter().map(|v| vec![v; 16]).collect();

        let cands = find_best_candidates(&frames, 10.0, 1.5, 5);
        assert!(!cands.is_empty());
        let best = &cands[0];
        assert!(best.similarity > 99.0, "expected >99%, got {}", best.similarity);
        assert!((best.end_sec - best.start_sec - 2.0).abs() < 0.05);
    }

    #[test]
    fn injects_libvpx_decoder_and_alpha_for_vp9_webm() {
        let probe = sample_probe(true, "vp9");
        let mut opts = base_opts(LoopMode::Crossfade, OutputFormat::Webm);
        opts.chroma_key = Some(ChromaKeyParams {
            color_hex: "#00FF00".to_string(),
            similarity: 0.3,
            blend: 0.1,
        });

        let (args, out, dur) = build_loop_ffmpeg_args(&opts, &probe).unwrap();
        assert_eq!(out, "/tmp/avatar_idle_loop.webm");
        assert!((dur - 2.5).abs() < 1e-3);
        let joined = args.join(" ");
        assert!(joined.contains("-c:v libvpx-vp9 -i"));
        assert!(joined.contains("xfade=transition=fade"));
        assert!(joined.contains("colorkey=0x00FF00:0.3000:0.1000"));
        assert!(joined.contains("-pix_fmt yuva420p"));
        assert!(joined.contains("-auto-alt-ref 0"));
    }

    #[test]
    fn builds_ping_pong_morph_and_smart_cut_graphs() {
        let probe = sample_probe(true, "vp9");

        // Ping-Pong with exact end frame
        let mut pp = base_opts(LoopMode::PingPong, OutputFormat::Mp4);
        pp.exact_end_frame = true;
        let (pp_args, _, _) = build_loop_ffmpeg_args(&pp, &probe).unwrap();
        let pp_joined = pp_args.join(" ");
        assert!(pp_joined.contains("reverse,trim=start_frame=1:end_frame=90"));
        assert!(pp_joined.contains("-c:v libx264 -pix_fmt yuv420p"));

        // Morph with alpha uses alphaextract + alphamerge + 3-frame minterpolate + scd=none
        let morph = base_opts(LoopMode::Morph, OutputFormat::Mov);
        let (m_args, m_out, m_dur) = build_loop_ffmpeg_args(&morph, &probe).unwrap();
        let m_joined = m_args.join(" ");
        assert_eq!(m_out, "/tmp/avatar_idle_loop.mov");
        assert!(m_dur > 3.0);
        assert!(m_joined.contains("concat=n=3:v=1:a=0,settb=1/1000,setpts=N*400"));
        assert!(m_joined.contains("alphaextract,minterpolate="));
        assert!(m_joined.contains("scd=none"));
        assert!(m_joined.contains("prores_ks"));

        // Smart-cut without crossfade + exact end frame -> appends frame 0
        let mut sc = base_opts(LoopMode::SmartCut, OutputFormat::Gif);
        sc.crossfade_sec = 0.0;
        sc.exact_end_frame = true;
        let (sc_args, sc_out, sc_dur) = build_loop_ffmpeg_args(&sc, &probe).unwrap();
        let sc_joined = sc_args.join(" ");
        assert_eq!(sc_out, "/tmp/avatar_idle_loop.gif");
        assert!((sc_dur - (3.0 + 1.0 / 30.0)).abs() < 1e-3);
        assert!(sc_joined.contains("[ex_body][ex_f0]concat=n=2:v=1:a=0[final]"));
        assert!(sc_joined.contains("palettegen=stats_mode=diff:reserve_transparent=on"));
    }
}
