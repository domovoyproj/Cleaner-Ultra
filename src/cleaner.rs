//! Allowlisted cache/TEMP roots. Each root has its own recoverable Space journal,
//! independent of the user's disk-analysis snapshot.
use crate::space::Space;
use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[derive(Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub category: String,
    pub label: String,
    pub root: PathBuf,
    pub age_seconds: u64,
    pub processes: Vec<String>,
}
fn normal(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().trim_start_matches(r"\\?\"))
}
// Expand DOS 8.3 aliases without resolving junctions as canonicalize() does.
// Windows can return a short TEMP path even when the profile uses a long name.
#[cfg(windows)]
fn long_form(path: &Path) -> Option<PathBuf> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Storage::FileSystem::GetLongPathNameW;
    let input: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut output = vec![0u16; 32768];
    let len = unsafe { GetLongPathNameW(input.as_ptr(), output.as_mut_ptr(), output.len() as u32) }
        as usize;
    (len > 0 && len < output.len())
        .then(|| normal(Path::new(&std::ffi::OsString::from_wide(&output[..len]))))
}
#[cfg(not(windows))]
fn long_form(path: &Path) -> Option<PathBuf> {
    Some(path.to_owned())
}
fn safe_root(root: &Path) -> bool {
    root.is_dir()
        && !root.is_symlink()
        && std::fs::canonicalize(root).is_ok_and(|p| normal(&p) == root)
}
fn rule(
    category: &str,
    label: String,
    path: PathBuf,
    age_seconds: u64,
    processes: &[&str],
) -> Option<Rule> {
    if !path.is_dir() || path.is_symlink() {
        return None;
    }
    let root = normal(&std::fs::canonicalize(&path).ok()?);
    if root.file_name().is_none() {
        return None;
    }
    // The expanded lexical path must still equal the resolved path. A junction
    // keeps its lexical name and therefore remains rejected by this comparison.
    let lexical = long_form(&path).unwrap_or_else(|| normal(&path));
    if lexical
        .to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_lowercase()
        != root
            .to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_lowercase()
    {
        return None;
    }
    let id = format!(
        "{:x}",
        Sha256::digest(format!("{category}:{}", root.to_string_lossy().to_lowercase()).as_bytes())
    );
    Some(Rule {
        id,
        category: category.into(),
        label,
        root,
        age_seconds,
        processes: processes.iter().map(|s| s.to_string()).collect(),
    })
}
fn children(path: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()))
        .take(2048)
        .map(|e| e.path())
        .collect()
}
pub fn discover() -> Vec<Rule> {
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default());
    let roaming = PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default());
    let mut rules = Vec::new();
    let mut add = |category: &str, label: String, path: PathBuf, age: u64, processes: &[&str]| {
        if let Some(r) = rule(category, label, path, age, processes) {
            rules.push(r);
        }
    };
    add(
        "user_temp",
        "Временные файлы пользователя · старше 48 часов".into(),
        std::env::temp_dir(),
        48 * 3600,
        &[],
    );
    if let Some(w) = std::env::var_os("SystemRoot") {
        add(
            "windows",
            "Windows TEMP · старше 48 часов".into(),
            PathBuf::from(w).join("Temp"),
            48 * 3600,
            &[],
        );
    }
    for (name, relative, process) in [
        ("Chrome", r"Google\Chrome\User Data", "chrome.exe"),
        ("Edge", r"Microsoft\Edge\User Data", "msedge.exe"),
        (
            "Яндекс Браузер",
            r"Yandex\YandexBrowser\User Data",
            "browser.exe",
        ),
        (
            "Brave",
            r"BraveSoftware\Brave-Browser\User Data",
            "brave.exe",
        ),
        ("Vivaldi", r"Vivaldi\User Data", "vivaldi.exe"),
    ] {
        for profile in children(&local.join(relative)).into_iter().filter(|p| {
            p.file_name()
                .is_some_and(|s| s == "Default" || s.to_string_lossy().starts_with("Profile "))
        }) {
            for cache in ["Cache", "Code Cache", "GPUCache"] {
                add(
                    "browsers",
                    format!(
                        "{name} · {} · {cache}",
                        profile.file_name().unwrap().to_string_lossy()
                    ),
                    profile.join(cache),
                    0,
                    &[process],
                );
            }
        }
    }
    for profile in children(&local.join(r"Mozilla\Firefox\Profiles")) {
        add(
            "browsers",
            format!(
                "Firefox · {} · HTTP-кеш",
                profile.file_name().unwrap().to_string_lossy()
            ),
            profile.join("cache2"),
            0,
            &["firefox.exe"],
        );
    }
    for base in [&local, &roaming] {
        for edition in ["Opera Stable", "Opera GX Stable"] {
            let home = base.join("Opera Software").join(edition);
            for profile in [home.clone(), home.join("Default")] {
                for cache in ["Cache", "Code Cache", "GPUCache"] {
                    add(
                        "browsers",
                        format!("{edition} · {cache}"),
                        profile.join(cache),
                        0,
                        &["opera.exe"],
                    );
                }
            }
        }
    }
    for (name, home, process) in [
        ("Discord", roaming.join("discord"), "discord.exe"),
        ("Slack", roaming.join("Slack"), "slack.exe"),
        ("VS Code", roaming.join("Code"), "code.exe"),
        ("Teams", roaming.join(r"Microsoft\Teams"), "teams.exe"),
    ] {
        for cache in ["Cache", "Code Cache", "GPUCache"] {
            add(
                "programs",
                format!("{name} · {cache}"),
                home.join(cache),
                0,
                &[process],
            );
        }
    }
    add(
        "programs",
        "DirectX · кеш шейдеров старше 7 дней".into(),
        local.join("D3DSCache"),
        7 * 86400,
        &[],
    );
    let mut ids = HashSet::new();
    rules.retain(|r| ids.insert(r.id.clone()));
    rules.sort_by(|a, b| a.label.cmp(&b.label));
    rules
}
#[cfg(windows)]
fn processes() -> Result<HashSet<String>> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::*,
    };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error().into());
    }
    struct Handle(windows_sys::Win32::Foundation::HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let h = Handle(snapshot);
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    let mut names = HashSet::new();
    if unsafe { Process32FirstW(h.0, &mut entry) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    loop {
        let len = entry
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.szExeFile.len());
        names.insert(String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase());
        if unsafe { Process32NextW(h.0, &mut entry) } == 0 {
            break;
        }
    }
    Ok(names)
}
#[cfg(not(windows))]
fn processes() -> Result<HashSet<String>> {
    Ok(HashSet::new())
}
fn blocked(rule: &Rule) -> Option<String> {
    if rule.category == "windows" && std::fs::read_dir(&rule.root).is_err() {
        return Some(
            "Для Windows TEMP перезапустите Cleaner Ultra с правами администратора".into(),
        );
    }
    if rule.processes.is_empty() {
        return None;
    }
    match processes() {
        Ok(names) => rule
            .processes
            .iter()
            .find(|p| names.contains(p.as_str()))
            .map(|p| format!("Закройте {p}, затем повторите проверку")),
        Err(_) => Some("Не удалось проверить работающие программы".into()),
    }
}
#[derive(Default, Clone, Serialize)]
struct Progress {
    state: String,
    job: String,
    current: String,
    rules_done: u64,
    rules_total: u64,
    files: u64,
    bytes: u64,
    warnings: Vec<String>,
}
pub struct Cleaner {
    data: PathBuf,
    db: Mutex<Connection>,
    spaces: Mutex<HashMap<String, Arc<Space>>>,
    progress: Mutex<Progress>,
    created: Mutex<Instant>,
    cancel: AtomicBool,
    mutation: Mutex<()>,
    exclusions: Mutex<Vec<String>>,
}
impl Cleaner {
    pub fn open(data: &Path) -> Result<Arc<Self>> {
        let data = data.join("cleanup");
        std::fs::create_dir_all(&data)?;
        let db = Connection::open(data.join("catalog.sqlite3"))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS rules(id TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS findings(id TEXT PRIMARY KEY,job TEXT,rule_id TEXT,file_id INTEGER,scan INTEGER,path TEXT,size INTEGER,label TEXT,category TEXT); CREATE INDEX IF NOT EXISTS cleanup_size ON findings(job,size DESC,id);")?;
        Ok(Arc::new(Self {
            data,
            db: Mutex::new(db),
            spaces: Mutex::new(HashMap::new()),
            progress: Mutex::new(Progress::default()),
            created: Mutex::new(Instant::now()),
            cancel: AtomicBool::new(false),
            mutation: Mutex::new(()),
            exclusions: Mutex::new(Vec::new()),
        }))
    }
    pub fn catalog(&self) -> Value {
        let items = discover()
            .into_iter()
            .map(|r| {
                let reason = blocked(&r);
                json!({"rule":r,"blocked":reason.is_some(),"reason":reason})
            })
            .collect::<Vec<_>>();
        json!({"items":items,"note":"Только кеш и TEMP. Cookies, пароли, история, закладки, сессии и данные сайтов не включаются."})
    }
    fn remember(&self, r: &Rule) -> Result<()> {
        self.db.lock().execute(
            "INSERT OR REPLACE INTO rules VALUES(?,?)",
            params![r.id, serde_json::to_string(r)?],
        )?;
        Ok(())
    }
    fn stored(&self, id: &str) -> Result<Rule> {
        let raw: String = self
            .db
            .lock()
            .query_row("SELECT value FROM rules WHERE id=?", [id], |r| r.get(0))
            .context("Категория не найдена")?;
        let rule: Rule = serde_json::from_str(&raw)?;
        if rule.id != id || id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("Неверный идентификатор категории");
        }
        Ok(rule)
    }
    fn space(&self, r: &Rule) -> Result<Arc<Space>> {
        if !safe_root(&r.root) {
            bail!("Каталог недоступен или содержит ссылку");
        }
        let mut spaces = self.spaces.lock();
        if let Some(s) = spaces.get(&r.id) {
            return Ok(s.clone());
        }
        let s = Space::open_cleanup(&self.data.join(&r.id), &r.root)?;
        spaces.insert(r.id.clone(), s.clone());
        Ok(s)
    }
    pub fn scan(self: &Arc<Self>, ids: &[String]) -> Result<Value> {
        self.scan_excluding(ids, &[])
    }
    pub fn scan_excluding(
        self: &Arc<Self>,
        ids: &[String],
        exclusions: &[String],
    ) -> Result<Value> {
        let rules = discover();
        let selected = ids
            .iter()
            .map(|id| {
                rules
                    .iter()
                    .find(|r| r.id == *id)
                    .cloned()
                    .context("Категория изменилась. Обновите режимы")
            })
            .collect::<Result<Vec<_>>>()?;
        self.scan_rules_excluding(selected, exclusions.to_vec())
    }
    #[cfg(test)]
    fn scan_rules(self: &Arc<Self>, rules: Vec<Rule>) -> Result<Value> {
        self.scan_rules_excluding(rules, Vec::new())
    }
    fn scan_rules_excluding(
        self: &Arc<Self>,
        rules: Vec<Rule>,
        exclusions: Vec<String>,
    ) -> Result<Value> {
        let _guard = self.mutation.lock();
        if self.progress.lock().state == "running" {
            bail!("Проверка очистки уже идёт");
        }
        if rules.is_empty() || rules.len() > 4096 {
            bail!("Выберите категории очистки");
        }
        let mut seen = HashSet::new();
        if rules.iter().any(|r| !seen.insert(&r.id)) {
            bail!("Повторяющиеся категории");
        }
        for r in &rules {
            self.remember(r)?;
        }
        *self.exclusions.lock() = exclusions.clone();
        let job = uuid::Uuid::new_v4().to_string();
        self.db.lock().execute("DELETE FROM findings", [])?;
        *self.progress.lock() = Progress {
            state: "running".into(),
            job: job.clone(),
            rules_total: rules.len() as u64,
            ..Default::default()
        };
        *self.created.lock() = Instant::now();
        self.cancel.store(false, Ordering::Release);
        let this = self.clone();
        std::thread::spawn(move || {
            for r in rules {
                if this.cancel.load(Ordering::Acquire) {
                    break;
                }
                this.progress.lock().current = r.label.clone();
                let result = (|| -> Result<()> {
                    if let Some(reason) = blocked(&r) {
                        bail!(reason);
                    }
                    let space = this.space(&r)?;
                    space.save_exclusions(&exclusions)?;
                    let scan = space.start(&r.root.to_string_lossy())?["id"]
                        .as_i64()
                        .context("Нет снимка")?;
                    loop {
                        let p = space.progress.lock().clone();
                        this.progress.lock().current =
                            format!("{} · файлов: {} · {}", r.label, p.files, p.current);
                        if p.state != "running" {
                            if p.state != "complete" {
                                bail!("Проверка категории не завершена");
                            }
                            if p.errors > 0 {
                                this.warning(format!(
                                    "{}: {} недоступных элементов",
                                    r.label, p.errors
                                ));
                            }
                            break;
                        }
                        if this.cancel.load(Ordering::Acquire) {
                            space.stop();
                        }
                        std::thread::sleep(Duration::from_millis(60));
                    }
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let mut offset = 0;
                    loop {
                        let page = space.files("all", "", offset, "", "")?;
                        let rows = page["files"].as_array().context("Нет файлов")?;
                        let mut db = this.db.lock();
                        let tx = db.transaction()?;
                        for f in rows {
                            let stamp = f["modified"].as_u64().unwrap_or(0);
                            if f["protected"] == true
                                || stamp == 0
                                || stamp.saturating_add(r.age_seconds) > now
                            {
                                continue;
                            }
                            let id = uuid::Uuid::new_v4().to_string();
                            tx.execute(
                                "INSERT INTO findings VALUES(?,?,?,?,?,?,?,?,?)",
                                params![
                                    id,
                                    job,
                                    r.id,
                                    f["id"].as_i64(),
                                    scan,
                                    f["path"].as_str(),
                                    f["size"].as_u64(),
                                    r.label,
                                    r.category
                                ],
                            )?;
                            let mut p = this.progress.lock();
                            p.files += 1;
                            p.bytes = p.bytes.saturating_add(f["size"].as_u64().unwrap_or(0));
                        }
                        tx.commit()?;
                        drop(db);
                        offset += rows.len() as i64;
                        if offset >= page["total"].as_i64().unwrap_or(0)
                            || rows.is_empty()
                            || this.cancel.load(Ordering::Acquire)
                        {
                            break;
                        }
                    }
                    Ok(())
                })();
                if let Err(e) = result {
                    this.warning(format!("{}: {e}", r.label));
                }
                this.progress.lock().rules_done += 1;
            }
            let mut p = this.progress.lock();
            p.state = if this.cancel.load(Ordering::Acquire) {
                "cancelled"
            } else {
                "complete"
            }
            .into();
            p.current.clear();
            *this.created.lock() = Instant::now();
        });
        Ok(json!({"job":self.progress.lock().job}))
    }
    fn warning(&self, text: String) {
        let mut p = self.progress.lock();
        if p.warnings.len() < 100 {
            p.warnings.push(text);
        }
    }
    pub fn status(&self) -> Value {
        json!(*self.progress.lock())
    }
    pub fn stop(&self) -> Value {
        self.cancel.store(true, Ordering::Release);
        json!({"stopping":true})
    }
    pub fn files(&self, offset: i64) -> Result<Value> {
        let p = self.progress.lock().clone();
        let db = self.db.lock();
        let offset = offset.max(0);
        let rows=db.prepare("SELECT id,path,size,label,category FROM findings WHERE job=? ORDER BY size DESC,id LIMIT 60 OFFSET ?")?.query_map(params![p.job,offset],|r|Ok(json!({"id":r.get::<_,String>(0)?,"path":r.get::<_,String>(1)?,"size":r.get::<_,u64>(2)?,"label":r.get::<_,String>(3)?,"category":r.get::<_,String>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!({"files":rows,"total":p.files,"offset":offset,"job":p.job}))
    }
    pub fn apply(&self, ids: &[String]) -> Result<Value> {
        self.apply_excluding(ids, &[])
    }
    pub fn apply_excluding(&self, ids: &[String], exclusions: &[String]) -> Result<Value> {
        let _guard = self.mutation.lock();
        let p = self.progress.lock().clone();
        if *self.exclusions.lock() != exclusions {
            bail!("Исключения изменились. Повторите проверку очистки");
        }
        if p.state == "running" || self.created.lock().elapsed().as_secs() > 900 {
            bail!("Повторите проверку очистки");
        }
        if ids.is_empty() || ids.len() > 500 {
            bail!("Выберите от 1 до 500 файлов");
        }
        let mut seen = HashSet::new();
        if ids.iter().any(|id| !seen.insert(id)) {
            bail!("Повторяющиеся файлы");
        }
        let mut groups: HashMap<String, Vec<(String, i64, i64, String)>> = HashMap::new();
        for id in ids {
            let row: (String, i64, i64, String) = self
                .db
                .lock()
                .query_row(
                    "SELECT rule_id,file_id,scan,path FROM findings WHERE id=? AND job=?",
                    params![id, p.job],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .context("Список изменился. Повторите проверку")?;
            groups
                .entry(row.0)
                .or_default()
                .push((id.clone(), row.1, row.2, row.3));
        }
        let mut done = 0;
        let mut moved = 0u64;
        let mut errors = Vec::new();
        for (rule_id, rows) in groups {
            let outcome = (|| -> Result<Value> {
                let r = self.stored(&rule_id)?;
                if let Some(reason) = blocked(&r) {
                    bail!(reason);
                }
                let space = self.space(&r)?;
                let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
                for (_, _, _, path) in &rows {
                    let meta = std::fs::metadata(path)?;
                    let age = meta.modified()?.duration_since(UNIX_EPOCH)?.as_secs();
                    if !Path::new(path).starts_with(&r.root)
                        || age.saturating_add(r.age_seconds) > now
                    {
                        bail!("Файл стал новым или вышел за границы категории");
                    }
                }
                space.quarantine(&rows.iter().map(|r| r.1).collect::<Vec<_>>(), rows[0].2)
            })();
            match outcome {
                Ok(result) => {
                    done += result["done"].as_u64().unwrap_or(0);
                    moved += result["moved_bytes"].as_u64().unwrap_or(0);
                    let failed = result["errors"].as_array().cloned().unwrap_or_default();
                    for (id, file_id, _, _) in rows {
                        if let Some(e) = failed.iter().find(|e| e["id"] == file_id) {
                            errors.push(json!({"id":id,"message":e["message"]}));
                        } else {
                            self.db
                                .lock()
                                .execute("DELETE FROM findings WHERE id=?", [id])?;
                        }
                    }
                }
                Err(e) => {
                    for (id, _, _, _) in rows {
                        errors.push(json!({"id":id,"message":e.to_string()}));
                    }
                }
            }
        }
        {
            let mut p = self.progress.lock();
            p.files = p.files.saturating_sub(done);
            p.bytes = p.bytes.saturating_sub(moved);
        }
        Ok(
            json!({"done":done,"moved_bytes":moved,"freed_bytes":0,"errors":errors,"note":"Файлы сохранены в восстанавливаемой корзине исходного диска. Свободное место пока не увеличилось."}),
        )
    }
    pub fn trash(&self, only: &str, offset: i64) -> Result<Value> {
        let rules = self
            .db
            .lock()
            .prepare("SELECT value FROM rules ORDER BY id")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut groups = Vec::new();
        for raw in rules {
            let r: Rule = serde_json::from_str(&raw)?;
            if !only.is_empty() && only != r.id {
                continue;
            }
            let space = match self.space(&r) {
                Ok(s) => s,
                Err(e) => {
                    groups.push(json!({"rule_id":r.id,"label":r.label,"error":e.to_string()}));
                    continue;
                }
            };
            let mut group = space.trash(if only.is_empty() { 0 } else { offset })?;
            if group["total"].as_u64().unwrap_or(0) > 0 {
                group["rule_id"] = json!(r.id);
                group["label"] = json!(r.label);
                groups.push(group);
            }
        }
        Ok(json!({"groups":groups}))
    }
    pub fn restore(&self, rule_id: &str, ids: &[i64]) -> Result<Value> {
        let _guard = self.mutation.lock();
        let r = self.stored(rule_id)?;
        if let Some(reason) = blocked(&r) {
            bail!(reason);
        }
        self.space(&r)?.restore(ids)
    }
    pub fn close(&self) {
        self.cancel.store(true, Ordering::Release);
        for space in self.spaces.lock().values() {
            space.stop();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn wait(c: &Arc<Cleaner>) {
        for _ in 0..1000 {
            if c.status()["state"] != "running" {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("cleanup scan timeout");
    }
    fn setup(age: u64) -> (tempfile::TempDir, Arc<Cleaner>, Rule, PathBuf, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let profile = t.path().join("profile");
        let cache = profile.join("Cache");
        std::fs::create_dir_all(&cache).unwrap();
        let a = cache.join("one.bin");
        let b = cache.join("keep.bin");
        std::fs::write(&a, b"own cache a").unwrap();
        std::fs::write(&b, b"own cache b").unwrap();
        std::fs::write(profile.join("Cookies"), b"keep cookies").unwrap();
        std::fs::write(profile.join("Login Data"), b"keep passwords").unwrap();
        let c = Cleaner::open(&t.path().join("data")).unwrap();
        let r = rule("browsers", "Own browser fixture".into(), cache, age, &[]).unwrap();
        (t, c, r, a, b)
    }
    #[test]
    fn category_root_accepts_a_trailing_separator_but_never_an_entire_drive() {
        let t = tempfile::tempdir().unwrap();
        let trailing = PathBuf::from(format!(
            "{}{}",
            t.path().display(),
            std::path::MAIN_SEPARATOR
        ));
        assert!(rule("user_temp", "own temp".into(), trailing, 0, &[]).is_some());
        #[cfg(windows)]
        assert!(
            rule(
                "user_temp",
                "forbidden whole drive".into(),
                PathBuf::from(r"C:\"),
                0,
                &[]
            )
            .is_none()
        );
    }
    #[cfg(windows)]
    #[test]
    fn category_accepts_dos_short_names_without_changing_the_resolved_root() {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("own-long-browser-cache-directory");
        std::fs::create_dir(&path).unwrap();
        let input: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut output = vec![0u16; 32768];
        let len =
            unsafe { GetShortPathNameW(input.as_ptr(), output.as_mut_ptr(), output.len() as u32) }
                as usize;
        assert!(len > 0 && len < output.len());
        let short = PathBuf::from(std::ffi::OsString::from_wide(&output[..len]));
        let r = rule("browsers", "Own short-path fixture".into(), short, 0, &[]).unwrap();
        assert_eq!(r.root, normal(&std::fs::canonicalize(path).unwrap()));
        assert!(safe_root(&r.root));
    }
    #[test]
    fn cleanup_respects_global_exclusions_and_rejects_changed_rules() {
        let (_t, c, r, a, b) = setup(0);
        let exclusions = vec![b.to_string_lossy().to_string()];
        c.scan_rules_excluding(vec![r], exclusions.clone()).unwrap();
        wait(&c);
        let rows = c.files(0).unwrap();
        assert_eq!(rows["total"], 1);
        assert_eq!(rows["files"][0]["path"], a.to_string_lossy().as_ref());
        let id = rows["files"][0]["id"].as_str().unwrap().to_owned();
        assert!(c.apply(&[id.clone()]).is_err());
        assert_eq!(c.apply_excluding(&[id], &exclusions).unwrap()["done"], 1);
        assert!(b.exists());
    }
    #[test]
    fn reviewed_cache_cleanup_survives_restart_and_preserves_credentials_and_exclusions() {
        let (t, c, r, a, b) = setup(0);
        c.scan_rules(vec![r.clone()]).unwrap();
        wait(&c);
        let rows = c.files(0).unwrap();
        assert_eq!(rows["total"], 2);
        let id = rows["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == a.to_string_lossy().as_ref())
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(c.apply(&[id.clone(), "invented".into()]).is_err());
        assert!(a.exists());
        assert_eq!(c.apply(&[id]).unwrap()["done"], 1);
        assert!(!a.exists());
        assert_eq!(std::fs::read(&b).unwrap(), b"own cache b");
        assert_eq!(
            std::fs::read(b.parent().unwrap().parent().unwrap().join("Cookies")).unwrap(),
            b"keep cookies"
        );
        c.close();
        drop(c);
        let c = Cleaner::open(&t.path().join("data")).unwrap();
        let trash = c.trash("", 0).unwrap();
        let stored = trash["groups"][0]["files"][0]["id"].as_i64().unwrap();
        assert_eq!(c.restore(&r.id, &[stored]).unwrap()["done"], 1);
        assert_eq!(std::fs::read(a).unwrap(), b"own cache a");
    }
    #[test]
    fn windows_temp_rule_only_includes_old_files_in_its_approved_root() {
        let (_t, c, mut r, a, b) = setup(48 * 3600);
        r.category = "windows".into();
        let old = SystemTime::now() - Duration::from_secs(72 * 3600);
        std::fs::File::options()
            .write(true)
            .open(&a)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(old))
            .unwrap();
        c.scan_rules(vec![r]).unwrap();
        wait(&c);
        let rows = c.files(0).unwrap();
        assert_eq!(rows["total"], 1);
        let id = rows["files"][0]["id"].as_str().unwrap().to_owned();
        assert_eq!(c.apply(&[id]).unwrap()["done"], 1);
        assert!(b.exists());
        assert!(!a.exists());
    }
    #[test]
    fn temp_age_and_revalidation_keep_new_or_changed_files() {
        let (_t, c, r, a, b) = setup(48 * 3600);
        let old = SystemTime::now() - Duration::from_secs(72 * 3600);
        std::fs::File::options()
            .write(true)
            .open(&a)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(old))
            .unwrap();
        c.scan_rules(vec![r]).unwrap();
        wait(&c);
        let rows = c.files(0).unwrap();
        assert_eq!(rows["total"], 1);
        let id = rows["files"][0]["id"].as_str().unwrap().to_owned();
        std::fs::write(&a, b"own file became recent").unwrap();
        let result = c.apply(&[id]).unwrap();
        assert_eq!(result["done"], 0);
        assert!(!result["errors"].as_array().unwrap().is_empty());
        assert!(b.exists());
        assert!(a.exists());
    }
    #[test]
    fn cleanup_index_does_not_replace_disk_snapshot_and_restore_refuses_conflict() {
        let (t, c, r, a, _b) = setup(0);
        let disk = Space::open(&t.path().join("disk-data")).unwrap();
        let source = t.path().join("user-files");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("own.txt"), b"keep disk snapshot").unwrap();
        let id = disk.start(&source.to_string_lossy()).unwrap()["id"]
            .as_i64()
            .unwrap();
        while disk.progress.lock().state == "running" {
            std::thread::sleep(Duration::from_millis(10));
        }
        c.scan_rules(vec![r.clone()]).unwrap();
        wait(&c);
        assert_eq!(disk.progress.lock().id, id);
        assert_eq!(disk.progress.lock().files, 1);
        let rows = c.files(0).unwrap();
        let id = rows["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == a.to_string_lossy().as_ref())
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        c.apply(&[id]).unwrap();
        let trash = c.trash("", 0).unwrap();
        let stored = trash["groups"][0]["files"][0]["id"].as_i64().unwrap();
        std::fs::write(&a, b"replacement must survive").unwrap();
        assert_eq!(c.restore(&r.id, &[stored]).unwrap()["done"], 0);
        assert_eq!(std::fs::read(a).unwrap(), b"replacement must survive");
    }
    #[cfg(windows)]
    #[test]
    fn application_started_after_scan_blocks_cleanup() {
        let (_t, c, mut r, a, _b) = setup(0);
        c.scan_rules(vec![r.clone()]).unwrap();
        wait(&c);
        let id = c.files(0).unwrap()["files"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        r.processes = vec![
            std::env::current_exe()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_lowercase(),
        ];
        c.remember(&r).unwrap();
        assert_eq!(c.apply(&[id]).unwrap()["done"], 0);
        assert!(a.exists());
    }
}
