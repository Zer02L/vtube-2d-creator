use std::env;
use std::process;

use tauri_app_lib::looper::{
    analyze_loop_points, process_loop_video, LoopMode, LoopProcessOptions, OutputFormat,
};

fn print_help() {
    eprintln!(
        "loop-video — CLI инструмент для создания бесшовных видео-циклов (creator-2d-avatar)\n\n\
        ИСПОЛЬЗОВАНИЕ:\n  \
          loop-video analyze <input> [--min-duration <sec>] [--top <n>]\n  \
          loop-video process <input> [Опции]\n\n\
        ОПЦИИ process:\n  \
          -o, --output <path>       Путь выходного файла (по умолчанию <name>_loop.<ext>)\n  \
          --mode <mode>             crossfade | ping-pong | morph | smart-cut (по умолч. crossfade)\n  \
          --start <sec>             Начало обрезки в секундах\n  \
          --end <sec>               Конец обрезки в секундах\n  \
          --auto                    Автоматически выбрать лучший цикл через analyze\n  \
          --crossfade <sec>         Длительность кроссфейда (по умолч. 0.45)\n  \
          --bridge <sec>            Длительность морфинг-моста (по умолч. 0.40)\n  \
          --exact-end               Сделать последний кадр попиксельно равным первому\n  \
          --format <fmt>            webm | mp4 | gif | mov (по умолч. webm)\n  \
          --fps <n>                 Целевой FPS\n  \
          --resolution <WxH>        Целевое разрешение (например 1920x1080)\n  \
          -h, --help                Показать эту справку"
    );
}

fn parse_mode(s: &str) -> Result<LoopMode, String> {
    match s.to_ascii_lowercase().as_str() {
        "crossfade" => Ok(LoopMode::Crossfade),
        "ping-pong" | "ping_pong" | "pingpong" => Ok(LoopMode::PingPong),
        "morph" => Ok(LoopMode::Morph),
        "smart-cut" | "smart_cut" | "smartcut" => Ok(LoopMode::SmartCut),
        other => Err(format!(
            "Неизвестный режим '{}'. Доступны: crossfade, ping-pong, morph, smart-cut",
            other
        )),
    }
}

fn parse_format(s: &str) -> Result<OutputFormat, String> {
    match s.to_ascii_lowercase().as_str() {
        "webm" => Ok(OutputFormat::Webm),
        "mp4" => Ok(OutputFormat::Mp4),
        "gif" => Ok(OutputFormat::Gif),
        "mov" => Ok(OutputFormat::Mov),
        other => Err(format!(
            "Неизвестный формат '{}'. Доступны: webm, mp4, gif, mov",
            other
        )),
    }
}

fn run_analyze(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err("Укажите путь к видео: loop-video analyze <input>".to_string());
    }
    let input = &args[0];
    let mut min_duration: Option<f64> = None;
    let mut top: usize = 5;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--min-duration" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| "Отсутствует значение для --min-duration".to_string())?;
                min_duration = Some(
                    v.parse::<f64>()
                        .map_err(|_| format!("Неверное число для --min-duration: {}", v))?,
                );
            }
            "--top" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| "Отсутствует значение для --top".to_string())?;
                top = v
                    .parse::<usize>()
                    .map_err(|_| format!("Неверное целое для --top: {}", v))?;
            }
            unknown => return Err(format!("Неизвестный аргумент: {}", unknown)),
        }
        i += 1;
    }

    let res = analyze_loop_points(input, min_duration, top)?;
    println!("Файл: {}", res.probe.path);
    println!(
        "Поток: {}x{}, {:.2} fps, {:.2}с ({} кадров), кодек={}, pix_fmt={}, alpha={}",
        res.probe.width,
        res.probe.height,
        res.probe.fps,
        res.probe.duration_sec,
        res.probe.frame_count,
        res.probe.codec_name,
        res.probe.pix_fmt,
        res.probe.has_alpha
    );
    println!(
        "Исходный стык (1-й <-> последний кадр): {:.2}%",
        res.original_seam_similarity
    );
    println!("\nЛучшие интервалы цикла (проанализировано {} кадров @ {:.1} fps):", res.sampled_frames, res.sampled_fps);
    for (idx, c) in res.candidates.iter().enumerate() {
        println!(
            "  #{}: {:.3}s -> {:.3}s (длительность {:.3}s) | общее совпадение {:.2}% (поза {:.2}%, движение {:.2}%)",
            idx + 1,
            c.start_sec,
            c.end_sec,
            c.duration_sec,
            c.similarity,
            c.pose_similarity,
            c.motion_similarity
        );
    }
    Ok(())
}

fn run_process(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err("Укажите путь к видео: loop-video process <input>".to_string());
    }
    let input = args[0].clone();
    let mut output_path: Option<String> = None;
    let mut mode = LoopMode::Crossfade;
    let mut start_sec: Option<f64> = None;
    let mut end_sec: Option<f64> = None;
    let mut auto_pick = false;
    let mut crossfade_sec = 0.45_f64;
    let mut bridge_sec = 0.40_f64;
    let mut exact_end_frame = false;
    let mut format = OutputFormat::Webm;
    let mut target_fps: Option<f64> = None;
    let mut resolution: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--output" => {
                i += 1;
                output_path = Some(
                    args.get(i)
                        .ok_or_else(|| "Отсутствует значение для --output".to_string())?
                        .clone(),
                );
            }
            "--mode" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| "Отсутствует значение для --mode".to_string())?;
                mode = parse_mode(v)?;
            }
            "--start" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| "Отсутствует значение для --start".to_string())?;
                start_sec = Some(
                    v.parse::<f64>()
                        .map_err(|_| format!("Неверное число для --start: {}", v))?,
                );
            }
            "--end" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| "Отсутствует значение для --end".to_string())?;
                end_sec = Some(
                    v.parse::<f64>()
                        .map_err(|_| format!("Неверное число для --end: {}", v))?,
                );
            }
            "--auto" => auto_pick = true,
            "--crossfade" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| "Отсутствует значение для --crossfade".to_string())?;
                crossfade_sec = v
                    .parse::<f64>()
                    .map_err(|_| format!("Неверное число для --crossfade: {}", v))?;
            }
            "--bridge" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| "Отсутствует значение для --bridge".to_string())?;
                bridge_sec = v
                    .parse::<f64>()
                    .map_err(|_| format!("Неверное число для --bridge: {}", v))?;
            }
            "--exact-end" => exact_end_frame = true,
            "--format" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| "Отсутствует значение для --format".to_string())?;
                format = parse_format(v)?;
            }
            "--fps" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| "Отсутствует значение для --fps".to_string())?;
                target_fps = Some(
                    v.parse::<f64>()
                        .map_err(|_| format!("Неверное число для --fps: {}", v))?,
                );
            }
            "--resolution" => {
                i += 1;
                resolution = Some(
                    args.get(i)
                        .ok_or_else(|| "Отсутствует значение для --resolution".to_string())?
                        .clone(),
                );
            }
            unknown => return Err(format!("Неизвестный аргумент: {}", unknown)),
        }
        i += 1;
    }

    if auto_pick && start_sec.is_none() && end_sec.is_none() {
        eprintln!("[auto] Поиск лучшего сегмента для зацикливания...");
        let analysis = analyze_loop_points(&input, None, 1)?;
        if let Some(best) = analysis.candidates.first() {
            eprintln!(
                "[auto] Выбран интервал {:.3}s -> {:.3}s (совпадение {:.2}%)",
                best.start_sec, best.end_sec, best.similarity
            );
            start_sec = Some(best.start_sec);
            end_sec = Some(best.end_sec);
        }
    }

    let opts = LoopProcessOptions {
        input_path: input,
        output_path,
        mode,
        start_sec,
        end_sec,
        crossfade_sec,
        bridge_sec,
        exact_end_frame,
        format,
        resolution,
        target_fps,
        chroma_key: None,
    };

    let result = process_loop_video(&opts, |ev| {
        eprintln!("[{:>5.1}%] {}: {}", ev.percent, ev.stage, ev.message);
    })?;

    println!("Готово: {}", result.output_path);
    println!(
        "Длительность: {:.3}s | Кадров: {} | Размер: {} байт",
        result.duration_sec, result.frame_count, result.file_size_bytes
    );
    println!(
        "Совпадение стыка (1-й <-> последний кадр): {:.2}% -> {:.2}%",
        result.original_seam_similarity, result.output_seam_similarity
    );
    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return;
    }

    let res = match args[0].as_str() {
        "analyze" => run_analyze(&args[1..]),
        "process" => run_process(&args[1..]),
        other => Err(format!(
            "Неизвестная команда '{}'. Используйте 'analyze' или 'process' (см. --help).",
            other
        )),
    };

    if let Err(err) = res {
        eprintln!("Ошибка: {}", err);
        process::exit(1);
    }
}
