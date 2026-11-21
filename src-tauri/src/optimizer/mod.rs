use std::process::Command;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChromaKeyParams {
    pub color_hex: String, // e.g. "0x00FF00"
    pub similarity: f32,   // 0.0 - 1.0
    pub blend: f32,        // 0.0 - 1.0
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TranscodeOptions {
    pub input_path: String,
    pub output_path: String,
    pub resolution: Option<String>, // e.g. "1920x1080", "1280x720"
    pub chroma_key: Option<ChromaKeyParams>,
}

pub struct VideoOptimizer;

impl VideoOptimizer {
    /// Builds the FFmpeg command line arguments for exporting transparent WebM VP9.
    pub fn build_ffmpeg_command(opts: &TranscodeOptions) -> Command {
        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y"); // overwrite output
        cmd.arg("-i").arg(&opts.input_path);

        // Video filters: chroma key + scaling
        let mut filters = Vec::new();

        if let Some(chroma) = &opts.chroma_key {
            // Clean hex color (ensure format 0xRRGGBB)
            let color = chroma.color_hex.trim_start_matches('#');
            filters.push(format!("colorkey=0x{}:{}:{}", color, chroma.similarity, chroma.blend));
        }

        if let Some(res) = &opts.resolution {
            filters.push(format!("scale={}:force_original_aspect_ratio=decrease", res));
        }

        if !filters.is_empty() {
            cmd.arg("-vf").arg(filters.join(","));
        }

        // WebM VP9 settings with true alpha channel
        cmd.args(&[
            "-c:v", "libvpx-vp9",
            "-pix_fmt", "yuva420p",
            "-b:v", "2M",
            "-auto-alt-ref", "0",
            "-an", // remove audio track for avatar loops
        ]);

        cmd.arg(&opts.output_path);
        cmd
    }
}
