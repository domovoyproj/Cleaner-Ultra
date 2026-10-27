use crate::analysis::{self, Description};
use anyhow::{Context, Result, bail};
use image::DynamicImage;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub const EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "avi", "webm", "wmv", "mts", "m2ts", "mpg", "mpeg",
];
pub fn is_video(path: &Path) -> bool {
    EXTENSIONS.contains(&analysis::extension(path).as_str())
}
pub fn tool(name: &str) -> Option<PathBuf> {
    let variable = format!("CLEANER_ULTRA_{}", name.to_uppercase());
    if let Some(path) = std::env::var_os(variable) {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let executable = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    [
        executable.join("ffmpeg").join(&name),
        PathBuf::from(".cleaner_ultra/tools/ffmpeg").join(name),
    ]
    .into_iter()
    .find(|path| path.is_file())
}
struct Temporary(PathBuf);
impl Temporary {
    fn new(scratch: &Path, extension: &str) -> Self {
        Self(scratch.join(format!("video-{}.{}", uuid::Uuid::new_v4(), extension)))
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn command(name: &str) -> Result<Command> {
    let mut command = Command::new(tool(name).with_context(|| {
        format!("Не найден {name}. Нужен полный комплект CleanerUltra с поддержкой видео")
    })?);
    command.args(["-v", "error", "-max_alloc", "67108864"]);
    if name == "ffmpeg" {
        command.args(["-nostdin", "-filter_threads", "1"]);
    }
    Ok(command)
}
#[derive(Debug)]
pub struct Metadata {
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub fps: f64,
    pub codec: String,
    pub captured: Option<f64>,
}
fn numeric(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|n| n.is_finite())
}
pub fn metadata(path: &Path, scratch: &Path) -> Result<Metadata> {
    let output = Temporary::new(scratch, "json");
    let mut probe = command("ffprobe")?;
    probe.args(["-protocol_whitelist", "file,pipe", "-select_streams", "V:0", "-show_entries", "stream=width,height,codec_name,avg_frame_rate,duration:stream_tags=creation_time,rotate:stream_side_data=rotation:format=duration:format_tags=creation_time", "-of", "json", "-o"])
        .arg(&output.0).arg(path);
    analysis::command_output(probe, 30)?;
    if fs::metadata(&output.0)?.len() > 1024 * 1024 {
        bail!("Метаданные видео слишком большие");
    }
    let value: Value = serde_json::from_slice(&fs::read(&output.0)?)?;
    let stream = value["streams"]
        .as_array()
        .and_then(|v| v.first())
        .context("В файле нет видеодорожки")?;
    let mut width = stream["width"].as_u64().unwrap_or(0) as u32;
    let mut height = stream["height"].as_u64().unwrap_or(0) as u32;
    let rotation = stream["side_data_list"]
        .as_array()
        .and_then(|v| v.iter().find_map(|s| numeric(&s["rotation"])))
        .or_else(|| numeric(&stream["tags"]["rotate"]))
        .unwrap_or(0.0);
    if (rotation.abs() % 180.0 - 90.0).abs() < 1.0 {
        std::mem::swap(&mut width, &mut height);
    }
    let duration = numeric(&stream["duration"])
        .or_else(|| numeric(&value["format"]["duration"]))
        .unwrap_or(0.0);
    if width == 0 || height == 0 || duration <= 0.0 {
        bail!("Не удалось определить размеры или длительность видео");
    }
    let fps = stream["avg_frame_rate"]
        .as_str()
        .and_then(|s| s.split_once('/'))
        .and_then(|(n, d)| Some(n.parse::<f64>().ok()? / d.parse::<f64>().ok()?))
        .filter(|n| n.is_finite() && *n >= 0.0)
        .unwrap_or(0.0);
    let captured = stream["tags"]["creation_time"]
        .as_str()
        .or_else(|| value["format"]["tags"]["creation_time"].as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp() as f64);
    Ok(Metadata {
        width,
        height,
        duration,
        fps,
        codec: stream["codec_name"].as_str().unwrap_or("").into(),
        captured,
    })
}
pub fn frame(path: &Path, scratch: &Path, time: f64) -> Result<DynamicImage> {
    let output = Temporary::new(scratch, "png");
    let mut decoder = command("ffmpeg")?;
    decoder
        .args(["-protocol_whitelist", "file,pipe", "-threads", "1", "-ss"])
        .arg(format!("{time:.4}"))
        .arg("-i")
        .arg(path)
        .args([
            "-map",
            "0:V:0",
            "-frames:v",
            "1",
            "-an",
            "-vf",
            "scale=640:480:force_original_aspect_ratio=decrease,setsar=1",
            "-threads",
            "1",
            "-y",
        ])
        .arg(&output.0);
    analysis::command_output(decoder, 60)?;
    image::open(&output.0).context("Не удалось извлечь кадр видео")
}
pub fn describe(path: &Path, thumb: &Path, scratch: &Path) -> Result<Description> {
    let metadata = metadata(path, scratch)?;
    let mut images = Vec::new();
    for fraction in [0.2, 0.5, 0.8] {
        images.push(frame(path, scratch, metadata.duration * fraction)?);
    }
    let mut description = analysis::describe_image(images[1].clone(), thumb, metadata.captured)?;
    description.dhash = format!("{:016x}", analysis::hashes(&images[0]).0);
    description.ahash = format!("{:016x}", analysis::hashes(&images[1]).0);
    description.crop_hash = format!("{:016x}", analysis::hashes(&images[2]).0);
    description.width = metadata.width;
    description.height = metadata.height;
    description.media_kind = "video".into();
    description.duration = metadata.duration;
    description.fps = metadata.fps;
    description.video_codec = metadata.codec;
    description.sharpness = 0.0;
    description.quality_hint.clear();
    Ok(description)
}
pub fn transcode(
    path: &Path,
    target: &Path,
    max_bytes: u64,
    cancel: impl Fn() -> bool,
) -> Result<()> {
    let mut encoder = command("ffmpeg")?;
    encoder
        .args(["-protocol_whitelist", "file,pipe", "-threads", "2", "-i"])
        .arg(path)
        .args([
            "-map",
            "0:V:0",
            "-map",
            "0:a:0?",
            "-vf",
            "scale=w='min(1280,iw)':h='min(720,ih)':force_original_aspect_ratio=decrease,setsar=1",
            "-c:v",
            "libvpx",
            "-b:v",
            "2M",
            "-deadline",
            "realtime",
            "-cpu-used",
            "5",
            "-threads",
            "2",
            "-c:a",
            "libopus",
            "-b:a",
            "96k",
            "-fs",
        ])
        .arg(max_bytes.to_string())
        .args(["-f", "webm", "-y"])
        .arg(target);
    analysis::command_output_until(encoder, 600, cancel)
}
pub fn mime(path: &Path) -> &'static str {
    match analysis::extension(path).as_str() {
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "wmv" => "video/x-ms-wmv",
        "mts" | "m2ts" => "video/mp2t",
        _ => "video/mpeg",
    }
}
