use anyhow::{Context, Result, bail};
use image::{DynamicImage, GenericImageView, imageops::FilterType};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const RAW: &[&str] = &[
    "cr2", "cr3", "nef", "nrw", "arw", "dng", "raf", "rw2", "orf", "pef", "srw", "raw",
];
pub fn extension(path: &Path) -> String {
    path.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
}
pub fn supported(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "tif" | "tiff" | "bmp" | "heic" | "heif" | "avif"
    ) || RAW.contains(&extension(path).as_str())
        || crate::video::is_video(path)
}
pub fn fingerprint(path: &Path) -> Result<String> {
    let mut source = BufReader::with_capacity(1024 * 1024, File::open(path)?);
    let mut digest = Sha256::new();
    let mut block = vec![0; 1024 * 1024];
    loop {
        let size = source.read(&mut block)?;
        if size == 0 {
            break;
        }
        digest.update(&block[..size]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
pub fn decoder_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CLEANER_ULTRA_MAGICK") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let executable = std::env::current_exe().ok()?.parent()?.to_path_buf();
    [
        executable.join("codecs/magick.exe"),
        PathBuf::from(".cleaner_ultra/tools/magick/magick.exe"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}
pub(crate) fn command_output(command: Command, seconds: u64) -> Result<()> {
    command_output_until(command, seconds, || false)
}
pub(crate) fn command_output_until(
    mut command: Command,
    seconds: u64,
    cancel: impl Fn() -> bool,
) -> Result<()> {
    command.stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn()?;
    let started = Instant::now();
    loop {
        if cancel() {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Подготовка видео отменена");
        }
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                bail!("Декодер не смог прочитать файл")
            }
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(seconds) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Время декодирования превышено")
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}
pub fn decode(path: &Path, scratch: &Path) -> Result<DynamicImage> {
    if !matches!(extension(path).as_str(), "heic" | "heif" | "avif")
        && !RAW.contains(&extension(path).as_str())
    {
        let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(384 * 1024 * 1024);
        reader.limits(limits);
        return reader.decode().context("Не удалось прочитать изображение");
    }
    let magick = decoder_path().context("Не найден локальный декодер HEIC/RAW")?;
    let temporary = scratch.join(format!("decode-{}.png", uuid::Uuid::new_v4()));
    let mut command = Command::new(magick);
    command
        .args([
            "-limit", "memory", "256MiB", "-limit", "map", "384MiB", "-limit", "disk", "512MiB",
            "-limit", "thread", "1",
        ])
        .arg(format!("{}[0]", path.display()))
        .args(["-auto-orient", "-resize", "8000x8000>"])
        .arg(&temporary);
    let result =
        command_output(command, 60).and_then(|_| image::open(&temporary).map_err(Into::into));
    let _ = std::fs::remove_file(temporary);
    result
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Description {
    pub width: u32,
    pub height: u32,
    pub captured: Option<f64>,
    pub dhash: String,
    pub ahash: String,
    pub crop_hash: String,
    pub sharpness: f64,
    pub brightness: f64,
    pub quality_hint: String,
    pub media_kind: String,
    pub duration: f64,
    pub fps: f64,
    pub video_codec: String,
}
pub(crate) fn hashes(image: &DynamicImage) -> (u64, u64) {
    let gray = image.resize_exact(9, 8, FilterType::Triangle).to_luma8();
    let mut d = 0;
    for y in 0..8 {
        for x in 0..8 {
            if gray.get_pixel(x, y)[0] > gray.get_pixel(x + 1, y)[0] {
                d |= 1 << (y * 8 + x)
            }
        }
    }
    let gray = image.resize_exact(8, 8, FilterType::Triangle).to_luma8();
    let mean = gray.as_raw().iter().map(|&x| x as u64).sum::<u64>() / 64;
    let a = gray.as_raw().iter().enumerate().fold(0, |hash, (i, &v)| {
        hash | ((u64::from(v as u64 >= mean)) << i)
    });
    (d, a)
}
pub fn describe(path: &Path, thumb: &Path, scratch: &Path) -> Result<Description> {
    if crate::video::is_video(path) {
        return crate::video::describe(path, thumb, scratch);
    }
    let metadata = File::open(path).ok().and_then(|file| {
        exif::Reader::new()
            .read_from_container(&mut BufReader::new(file))
            .ok()
    });
    let captured = metadata
        .as_ref()
        .and_then(|m| {
            m.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)
                .or_else(|| m.get_field(exif::Tag::DateTime, exif::In::PRIMARY))
        })
        .and_then(|f| {
            if let exif::Value::Ascii(values) = &f.value {
                values
                    .first()
                    .map(|v| String::from_utf8_lossy(v).to_string())
            } else {
                None
            }
        })
        .and_then(|s| {
            chrono::NaiveDateTime::parse_from_str(s.trim_end_matches('\0'), "%Y:%m:%d %H:%M:%S")
                .ok()
        })
        .map(|d| d.and_utc().timestamp() as f64);
    let orientation = metadata
        .as_ref()
        .and_then(|m| m.get_field(exif::Tag::Orientation, exif::In::PRIMARY))
        .and_then(|f| f.value.get_uint(0))
        .unwrap_or(1);
    let mut image = decode(path, scratch)?;
    if !RAW.contains(&extension(path).as_str())
        && !matches!(extension(path).as_str(), "heic" | "heif" | "avif")
    {
        image = orient(image, orientation)
    }
    describe_image(image, thumb, captured)
}
pub(crate) fn describe_image(
    image: DynamicImage,
    thumb: &Path,
    captured: Option<f64>,
) -> Result<Description> {
    let (width, height) = image.dimensions();
    let (dhash, ahash) = hashes(&image);
    let crop = image.crop_imm(
        width / 10,
        height / 10,
        (width * 8 / 10).max(1),
        (height * 8 / 10).max(1),
    );
    let (_, crop_hash) = hashes(&crop);
    let gray = image.thumbnail(256, 256).to_luma8();
    let (w, h) = gray.dimensions();
    let mut sum = 0.0;
    let mut squares = 0.0;
    let mut count = 0.0;
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let v = gray.get_pixel(x - 1, y)[0] as f64
                + gray.get_pixel(x + 1, y)[0] as f64
                + gray.get_pixel(x, y - 1)[0] as f64
                + gray.get_pixel(x, y + 1)[0] as f64
                - 4.0 * gray.get_pixel(x, y)[0] as f64;
            sum += v;
            squares += v * v;
            count += 1.0
        }
    }
    let sharpness = if count > 0.0 {
        squares / count - (sum / count).powi(2)
    } else {
        0.0
    };
    let brightness = gray.as_raw().iter().map(|&v| v as f64).sum::<f64>() / (w * h).max(1) as f64;
    let mut reasons = Vec::new();
    if sharpness < 45.0 {
        reasons.push("Возможный смаз")
    }
    if brightness < 35.0 {
        reasons.push("Тёмный кадр")
    }
    if brightness > 225.0 {
        reasons.push("Светлый кадр")
    }
    save_thumbnail(&image, thumb)?;
    Ok(Description {
        width,
        height,
        captured,
        dhash: format!("{dhash:016x}"),
        ahash: format!("{ahash:016x}"),
        crop_hash: format!("{crop_hash:016x}"),
        sharpness,
        brightness,
        quality_hint: reasons.join(" · "),
        media_kind: "photo".into(),
        duration: 0.0,
        fps: 0.0,
        video_codec: String::new(),
    })
}
pub(crate) fn save_thumbnail(image: &DynamicImage, thumb: &Path) -> Result<()> {
    let temporary = thumb.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    image
        .thumbnail(640, 480)
        .to_rgb8()
        .save_with_format(&temporary, image::ImageFormat::Jpeg)?;
    std::fs::rename(&temporary, thumb).or_else(|error| {
        if thumb.exists() {
            let _ = std::fs::remove_file(&temporary);
            Ok(())
        } else {
            Err(error)
        }
    })?;
    Ok(())
}
pub fn orient(image: DynamicImage, orientation: u32) -> DynamicImage {
    match orientation {
        2 => image.fliph(),
        3 => image.rotate180(),
        4 => image.flipv(),
        5 => image.rotate90().fliph(),
        6 => image.rotate90(),
        7 => image.rotate270().fliph(),
        8 => image.rotate270(),
        _ => image,
    }
}
pub fn preview(path: &Path, scratch: &Path) -> Result<Vec<u8>> {
    if crate::video::is_video(path) {
        let metadata = crate::video::metadata(path, scratch)?;
        let image = crate::video::frame(path, scratch, metadata.duration * 0.5)?.to_rgb8();
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 94).encode_image(&image)?;
        return Ok(bytes);
    }
    let metadata = File::open(path).ok().and_then(|f| {
        exif::Reader::new()
            .read_from_container(&mut BufReader::new(f))
            .ok()
    });
    let orientation = metadata
        .and_then(|m| {
            m.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                .and_then(|f| f.value.get_uint(0))
        })
        .unwrap_or(1);
    let mut image = decode(path, scratch)?;
    if !RAW.contains(&extension(path).as_str())
        && !matches!(extension(path).as_str(), "heic" | "heif" | "avif")
    {
        image = orient(image, orientation)
    }
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 94).encode_image(&image)?;
    Ok(bytes)
}
#[derive(Default)]
pub struct BKTree {
    nodes: Vec<(u64, i64, HashMap<u32, usize>)>,
}

pub struct HammingIndex {
    radius: u32,
    segments: Vec<(u32, u64, HashMap<u64, Vec<(u64, i64)>>)>,
}
impl HammingIndex {
    pub fn new(radius: u32) -> Self {
        let radius = radius.min(63);
        let count = radius + 1;
        let segments = (0..count)
            .map(|i| {
                let start = i * 64 / count;
                let bits = (i + 1) * 64 / count - start;
                let mask = if bits == 64 {
                    u64::MAX
                } else {
                    (1u64 << bits) - 1
                };
                (start, mask, HashMap::new())
            })
            .collect();
        Self { radius, segments }
    }
    pub fn add(&mut self, value: u64, id: i64) {
        for (shift, mask, buckets) in &mut self.segments {
            buckets
                .entry((value >> *shift) & *mask)
                .or_default()
                .push((value, id));
        }
    }
    pub fn find(&self, value: u64) -> Vec<i64> {
        let mut found = HashSet::new();
        // At most r changed bits leave at least one of r+1 disjoint segments intact.
        // Matching a segment selects candidates; full XOR/popcount verifies every result.
        for (shift, mask, buckets) in &self.segments {
            if let Some(candidates) = buckets.get(&((value >> *shift) & *mask)) {
                for &(hash, id) in candidates {
                    if (hash ^ value).count_ones() <= self.radius {
                        found.insert(id);
                    }
                }
            }
        }
        found.into_iter().collect()
    }
}
impl BKTree {
    pub fn add(&mut self, value: u64, id: i64) {
        if self.nodes.is_empty() {
            self.nodes.push((value, id, HashMap::new()));
            return;
        }
        let mut i = 0;
        loop {
            let distance = (value ^ self.nodes[i].0).count_ones();
            if distance == 0 {
                return;
            }
            if let Some(&next) = self.nodes[i].2.get(&distance) {
                i = next
            } else {
                let next = self.nodes.len();
                self.nodes[i].2.insert(distance, next);
                self.nodes.push((value, id, HashMap::new()));
                return;
            }
        }
    }
    pub fn find(&self, value: u64, radius: u32) -> Vec<i64> {
        let mut stack = if self.nodes.is_empty() {
            vec![]
        } else {
            vec![0]
        };
        let mut results = vec![];
        while let Some(i) = stack.pop() {
            let node = &self.nodes[i];
            let distance = (value ^ node.0).count_ones();
            if distance <= radius {
                results.push(node.1)
            }
            for (&edge, &child) in &node.2 {
                if edge >= distance.saturating_sub(radius) && edge <= distance + radius {
                    stack.push(child)
                }
            }
        }
        results
    }
}
