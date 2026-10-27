//! In-process HDD defragmentation using the documented Windows FSCTL interface.
//! Windows owns cluster relocation; Cleaner Ultra schedules and verifies each file.
use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Default, Serialize)]
pub struct Progress {
    pub state: String,
    pub mode: String,
    pub drive: String,
    pub current: String,
    pub checked: u64,
    pub total: u64,
    pub fragmented: u64,
    pub moved: u64,
    pub verified_bytes: u64,
    pub skipped: u64,
    pub errors: Vec<String>,
    pub elapsed_ms: u64,
}
pub struct Optimizer {
    progress: Mutex<Progress>,
    cancel: AtomicBool,
    history: Option<Arc<crate::maintenance::Maintenance>>,
}
impl Default for Optimizer {
    fn default() -> Self {
        Self {
            progress: Mutex::new(Progress::default()),
            cancel: AtomicBool::new(false),
            history: None,
        }
    }
}
impl Optimizer {
    pub fn with_history(history: Arc<crate::maintenance::Maintenance>) -> Self {
        Self {
            history: Some(history),
            ..Default::default()
        }
    }

    pub fn start_trim(self: &Arc<Self>, drive: &str) -> Result<Value> {
        let drive = crate::trim::validate_drive(drive)?;
        if !crate::space::is_administrator() {
            bail!("Перезапустите Cleaner Ultra с правами администратора");
        }
        let mut progress = self.progress.lock();
        if progress.state == "running" {
            bail!("Операция уже выполняется");
        }
        *progress = Progress {
            state: "running".into(),
            mode: "trim".into(),
            drive: drive.clone(),
            current: "Windows Storage API: ReTrim свободных блоков".into(),
            ..Default::default()
        };
        drop(progress);
        let this = self.clone();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let result = crate::trim::retrim(&drive);
            let mut p = this.progress.lock();
            p.state = if result.is_ok() { "complete" } else { "error" }.into();
            if let Err(e) = result {
                p.errors.push(e.to_string());
            }
            p.current.clear();
            p.elapsed_ms = started.elapsed().as_millis() as u64;
            if let Some(history) = &this.history {
                if let Err(e) = history.record("/api/optimizer/completed", &json!(*p)) {
                    p.errors.push(format!("История: {e}"));
                }
            }
        });
        Ok(json!({"started":true,"mode":"trim"}))
    }
    pub fn status(&self) -> Value {
        json!(*self.progress.lock())
    }
    pub fn stop(&self) -> Value {
        let p = self.progress.lock();
        if p.state == "running" && p.mode == "trim" {
            return json!({"stopping":false,"message":"Windows ReTrim не поддерживает отмену после запуска. Ожидайте результата."});
        }
        drop(p);
        self.cancel.store(true, Ordering::Release);
        json!({"stopping":true})
    }
    pub fn start(
        self: &Arc<Self>,
        drive: &str,
        analyze: bool,
        paths: Vec<String>,
    ) -> Result<Value> {
        self.start_paths(
            drive,
            analyze,
            paths.len() as u64,
            Box::new(paths.into_iter().map(Ok)),
        )
    }
    pub fn start_snapshot(
        self: &Arc<Self>,
        drive: &str,
        analyze: bool,
        paths: crate::space::OptimizationPaths,
    ) -> Result<Value> {
        self.start_paths(drive, analyze, paths.total, Box::new(paths))
    }
    fn start_paths(
        self: &Arc<Self>,
        drive: &str,
        analyze: bool,
        total: u64,
        paths: Box<dyn Iterator<Item = Result<String>> + Send>,
    ) -> Result<Value> {
        if drive.len() != 2
            || !drive.as_bytes()[0].is_ascii_alphabetic()
            || drive.as_bytes()[1] != b':'
        {
            bail!("Выберите локальный диск");
        }
        if total == 0 {
            bail!("Сначала проанализируйте диск или папку на этом диске");
        }
        let mut progress = self.progress.lock();
        if progress.state == "running" {
            bail!("Оптимизация уже идёт");
        }
        if !crate::space::is_administrator() {
            bail!("Перезапустите Cleaner Ultra с правами администратора");
        }
        let drive = drive.to_ascii_uppercase();
        // Probe before reporting a started job. Unknown media and SSD are never moved.
        let device = native::Device::open(&drive, !analyze)?;
        *progress = Progress {
            state: "running".into(),
            mode: if analyze { "analyze" } else { "defragment" }.into(),
            drive: drive.clone(),
            total,
            ..Default::default()
        };
        self.cancel.store(false, Ordering::Release);
        drop(progress);
        let this = self.clone();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let mut fatal = false;
            for path in paths {
                if this.cancel.load(Ordering::Acquire) {
                    break;
                }
                let path = match path {
                    Ok(p) => p,
                    Err(e) => {
                        this.progress.lock().errors.push(format!("Снимок: {e}"));
                        fatal = true;
                        break;
                    }
                };
                this.progress.lock().current = path.clone();
                let result = device.process(&path, !analyze, &this.cancel);
                let mut p = this.progress.lock();
                p.checked += 1;
                match result {
                    Ok(Some((fragmented, moved, bytes))) => {
                        p.fragmented += u64::from(fragmented);
                        p.moved += u64::from(moved);
                        p.verified_bytes += bytes;
                    }
                    Ok(None) => p.skipped += 1,
                    Err(e) => {
                        if e.to_string().contains("SHA-256 после") {
                            fatal = true;
                        }
                        p.skipped += 1;
                        if p.errors.len() < 100 {
                            p.errors.push(format!("{path}: {e}"));
                        }
                    }
                }
                p.elapsed_ms = started.elapsed().as_millis() as u64;
                if fatal {
                    break;
                }
            }
            let mut p = this.progress.lock();
            p.state = if fatal {
                "error"
            } else if this.cancel.load(Ordering::Acquire) {
                "cancelled"
            } else {
                "complete"
            }
            .into();
            p.current.clear();
            p.elapsed_ms = started.elapsed().as_millis() as u64;
            if let Some(history) = &this.history {
                if let Err(e) = history.record("/api/optimizer/completed", &json!(*p)) {
                    p.errors.push(format!("Не удалось сохранить историю: {e}"));
                }
            }
        });
        Ok(json!({"started":true}))
    }
}
/// Find a contiguous free run. Used on a fresh bitmap before every relocation.
fn free_run(bitmap: &[u8], bits: u64, needed: u64) -> Option<u64> {
    if needed == 0 {
        return None;
    }
    let mut run = 0;
    for bit in 0..bits.min(bitmap.len() as u64 * 8) {
        if bitmap[bit as usize / 8] & (1 << (bit % 8)) == 0 {
            run += 1;
            if run == needed {
                return Some(bit + 1 - needed);
            }
        } else {
            run = 0;
        }
    }
    None
}
#[cfg(not(windows))]
mod native {
    use super::*;
    pub struct Device;
    impl Device {
        pub fn open(_: &str, _: bool) -> Result<Self> {
            bail!("Только Windows")
        }
        pub fn process(
            &self,
            _: &str,
            _: bool,
            _: &AtomicBool,
        ) -> Result<Option<(bool, bool, u64)>> {
            bail!("Только Windows")
        }
    }
}
#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        fs::{File, OpenOptions},
        os::windows::{
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawHandle,
        },
        path::Path,
        ptr::null_mut,
    };
    use windows_sys::Win32::{
        Foundation::{ERROR_HANDLE_EOF, ERROR_MORE_DATA},
        Storage::FileSystem::{
            FILE_ATTRIBUTE_COMPRESSED, FILE_ATTRIBUTE_ENCRYPTED, FILE_ATTRIBUTE_REPARSE_POINT,
            FILE_ATTRIBUTE_SPARSE_FILE, FILE_ATTRIBUTE_SYSTEM, FILE_SHARE_READ, FILE_SHARE_WRITE,
            GetVolumeInformationW,
        },
        System::{IO::DeviceIoControl, Ioctl::*},
    };
    pub struct Device {
        volume: File,
        drive: String,
    }
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    fn ioctl(
        file: &File,
        code: u32,
        input: *const std::ffi::c_void,
        len: u32,
        output: &mut [u8],
    ) -> Result<(usize, bool)> {
        let mut returned = 0;
        let ok = unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                code,
                input,
                len,
                output.as_mut_ptr().cast(),
                output.len() as u32,
                &mut returned,
                null_mut(),
            )
        };
        if ok == 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(ERROR_MORE_DATA as i32) {
                return Ok((returned as usize, true));
            }
            if e.raw_os_error() == Some(ERROR_HANDLE_EOF as i32) {
                return Ok((0, false));
            }
            return Err(e.into());
        }
        Ok((returned as usize, false))
    }
    fn i64_at(bytes: &[u8], offset: usize) -> Result<i64> {
        Ok(i64::from_le_bytes(
            bytes
                .get(offset..offset + 8)
                .context("Неполный ответ диска")?
                .try_into()?,
        ))
    }
    impl Device {
        pub fn open(drive: &str, moving: bool) -> Result<Self> {
            let root = wide(&format!("{drive}\\"));
            let mut fs = [0u16; 32];
            if unsafe {
                GetVolumeInformationW(
                    root.as_ptr(),
                    null_mut(),
                    0,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    fs.as_mut_ptr(),
                    fs.len() as u32,
                )
            } == 0
            {
                bail!("Не удалось прочитать файловую систему");
            }
            if String::from_utf16_lossy(&fs).trim_end_matches('\0') != "NTFS" {
                bail!("Встроенный движок поддерживает NTFS");
            }
            let volume = OpenOptions::new()
                .read(true)
                .write(moving)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .open(format!(r"\\.\{drive}"))?;
            let query = STORAGE_PROPERTY_QUERY {
                PropertyId: StorageDeviceSeekPenaltyProperty,
                QueryType: PropertyStandardQuery,
                ..Default::default()
            };
            let mut response = [0u8; 64];
            let (size, _) = ioctl(
                &volume,
                IOCTL_STORAGE_QUERY_PROPERTY,
                (&query as *const STORAGE_PROPERTY_QUERY).cast(),
                std::mem::size_of_val(&query) as u32,
                &mut response,
            )?;
            if size < 9 {
                bail!("Тип носителя не определён");
            }
            if moving && response[8] == 0 {
                bail!("SSD или носитель без задержки поиска: выберите TRIM вместо дефрагментации");
            }
            Ok(Self {
                volume,
                drive: drive.into(),
            })
        }
        fn extents(&self, file: &File) -> Result<Option<(u32, u64)>> {
            let vcn = 0i64;
            let mut output = vec![0u8; 1024 * 1024];
            let (size, more) = ioctl(
                file,
                FSCTL_GET_RETRIEVAL_POINTERS,
                (&vcn as *const i64).cast(),
                8,
                &mut output,
            )?;
            if size == 0 {
                return Ok(None);
            }
            if more {
                bail!("Слишком много экстентов — файл пропущен");
            }
            if size < 16 {
                bail!("Неполная карта экстентов");
            }
            let count = u32::from_le_bytes(output[..4].try_into()?);
            if count == 0 {
                return Ok(None);
            }
            if 16 + count as usize * 16 > size || i64_at(&output, 8)? != 0 {
                bail!("Некорректная карта экстентов");
            }
            let mut previous = 0;
            let mut physical_end = None;
            let mut fragments = 0;
            for i in 0..count as usize {
                let next = i64_at(&output, 16 + i * 16)?;
                let lcn = i64_at(&output, 24 + i * 16)?;
                if next <= previous || lcn < 0 {
                    return Ok(None);
                }
                if physical_end != Some(lcn) {
                    fragments += 1;
                }
                physical_end = Some(lcn + next - previous);
                previous = next;
            }
            Ok(Some((fragments, previous as u64)))
        }
        fn destination(&self, clusters: u64, cancel: &AtomicBool) -> Result<Option<u64>> {
            self.destination_after(clusters, cancel, 0)
        }
        fn destination_after(
            &self,
            clusters: u64,
            cancel: &AtomicBool,
            mut start: i64,
        ) -> Result<Option<u64>> {
            loop {
                if cancel.load(Ordering::Acquire) {
                    return Ok(None);
                }
                let mut bitmap = vec![0u8; 1024 * 1024 + 16];
                let (size, more) = ioctl(
                    &self.volume,
                    FSCTL_GET_VOLUME_BITMAP,
                    (&start as *const i64).cast(),
                    8,
                    &mut bitmap,
                )?;
                if size < 16 {
                    bail!("Неполная карта свободных кластеров");
                }
                let base = i64_at(&bitmap, 0)?;
                let remaining = i64_at(&bitmap, 8)?;
                if base < 0 || remaining < 0 {
                    bail!("Некорректная карта тома");
                }
                let bits = ((size - 16) as u64 * 8).min(remaining as u64);
                if let Some(offset) = free_run(&bitmap[16..size], bits, clusters) {
                    return Ok(Some(base as u64 + offset));
                }
                if !more {
                    return Ok(None);
                }
                let next = base
                    .checked_add(bits as i64)
                    .context("Переполнение карты тома")?;
                if next <= start {
                    bail!("Карта тома не продвигается");
                }
                start = next;
            }
        }
        pub fn process(
            &self,
            path: &str,
            moving: bool,
            cancel: &AtomicBool,
        ) -> Result<Option<(bool, bool, u64)>> {
            // Pin this exact file, deny concurrent writes, renames and delete access.
            let file = OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ)
                .open(path)?;
            let meta = file.metadata()?;
            if !meta.is_file()
                || meta.file_attributes()
                    & (FILE_ATTRIBUTE_SPARSE_FILE
                        | FILE_ATTRIBUTE_COMPRESSED
                        | FILE_ATTRIBUTE_REPARSE_POINT
                        | FILE_ATTRIBUTE_ENCRYPTED
                        | FILE_ATTRIBUTE_SYSTEM)
                    != 0
            {
                return Ok(None);
            }
            let canonical = std::fs::canonicalize(path)?;
            let normal = canonical
                .to_string_lossy()
                .trim_start_matches(r"\\?\")
                .to_string();
            let requested = path.trim_start_matches(r"\\?\");
            if !normal.eq_ignore_ascii_case(requested) {
                bail!("Путь содержит ссылку или изменился");
            }
            if !normal
                .to_ascii_uppercase()
                .starts_with(&format!("{}\\", self.drive))
            {
                bail!("Путь покинул выбранный том");
            }
            let Some((fragments, clusters)) = self.extents(&file)? else {
                return Ok(None);
            };
            if !moving || fragments <= 1 {
                return Ok(Some((fragments > 1, false, 0)));
            }
            if meta.len() > 1024 * 1024 * 1024 || clusters > u32::MAX as u64 {
                return Ok(None);
            }
            let hash = crate::analysis::fingerprint(Path::new(path))?;
            let Some(lcn) = self.destination(clusters, cancel)? else {
                return Ok(None);
            };
            if cancel.load(Ordering::Acquire) {
                return Ok(None);
            }
            let request = MOVE_FILE_DATA {
                FileHandle: file.as_raw_handle(),
                StartingVcn: 0,
                StartingLcn: lcn as i64,
                ClusterCount: clusters as u32,
            };
            ioctl(
                &self.volume,
                FSCTL_MOVE_FILE,
                (&request as *const MOVE_FILE_DATA).cast(),
                std::mem::size_of_val(&request) as u32,
                &mut [],
            )?;
            let after = crate::analysis::fingerprint(Path::new(path))?;
            if after != hash {
                bail!("SHA-256 после перемещения не совпал. Остановите оптимизацию");
            }
            let final_fragments = self
                .extents(&file)?
                .context("Не удалось проверить результат")?
                .0;
            if final_fragments >= fragments {
                bail!("Число фрагментов не уменьшилось");
            }
            Ok(Some((true, true, meta.len())))
        }
    }
    #[test]
    #[ignore = "Requires explicit admin execution; operates only on its own uniquely named fixture"]
    fn native_cluster_relocation_own_fixture() {
        assert!(crate::space::is_administrator(), "Administrator required");
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join(".cleaner_ultra/native-qa");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("clusters-")
            .tempdir_in(&base)
            .unwrap();
        let path = dir.path().join("own-fixture.bin");
        let mut bytes = vec![0u8; 16 * 1024 * 1024];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (i.wrapping_mul(31).wrapping_add(i / 4096) % 251) as u8;
        }
        std::fs::write(&path, &bytes).unwrap();
        let path = std::fs::canonicalize(&path)
            .unwrap()
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_string();
        let drive = path[..2].to_ascii_uppercase();
        let volume = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(format!(r"\\.\{drive}"))
            .unwrap();
        let device = Device { volume, drive };
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
            .unwrap();
        let (_, clusters) = device.extents(&file).unwrap().unwrap();
        assert!(clusters > 2);
        let half = clusters / 2;
        let cancel = AtomicBool::new(false);
        let first = device
            .destination(clusters, &cancel)
            .unwrap()
            .expect("Free space required");
        let second = device
            .destination_after(clusters, &cancel, (first + clusters + 4096) as i64)
            .unwrap()
            .expect("Second free range required");
        for (vcn, lcn, count) in [(0, first, half), (half, second, clusters - half)] {
            let request = MOVE_FILE_DATA {
                FileHandle: file.as_raw_handle(),
                StartingVcn: vcn as i64,
                StartingLcn: lcn as i64,
                ClusterCount: count as u32,
            };
            ioctl(
                &device.volume,
                FSCTL_MOVE_FILE,
                (&request as *const MOVE_FILE_DATA).cast(),
                std::mem::size_of_val(&request) as u32,
                &mut [],
            )
            .unwrap();
        }
        assert!(device.extents(&file).unwrap().unwrap().0 >= 2);
        drop(file);
        let before = crate::analysis::fingerprint(Path::new(&path)).unwrap();
        let result = device.process(&path, true, &cancel).unwrap().unwrap();
        assert!(result.0 && result.1);
        assert_eq!(result.2, bytes.len() as u64);
        assert_eq!(
            crate::analysis::fingerprint(Path::new(&path)).unwrap(),
            before
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        println!("native relocation verified: own 16 MiB fixture, SHA-256 preserved");
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn free_bitmap_never_selects_allocated_clusters_or_padding() {
        assert_eq!(free_run(&[0b11000011], 8, 4), Some(2));
        assert_eq!(free_run(&[0b11000011], 8, 5), None);
        assert_eq!(free_run(&[0], 3, 4), None);
        assert_eq!(free_run(&[0], 8, 0), None);
        assert_eq!(free_run(&[255, 0], 16, 8), Some(8));
    }
    #[test]
    fn optimizer_rejects_command_injection_before_accessing_disk() {
        let o = Arc::new(Optimizer::default());
        for drive in ["C: & anything", "\\\\server", "C:\\", "-x", ""] {
            assert!(o.start(drive, false, vec!["test".into()]).is_err());
        }
    }
}
