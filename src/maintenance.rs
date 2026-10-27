//! Separate, recoverable startup management and current-user registry issue repair.
//! Only allowlisted values are changed; never delete keys, services or HKLM.
use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, path::Path, time::Instant};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_ONCE: &str = r"Software\Microsoft\Windows\CurrentVersion\RunOnce";
const LIMIT: usize = 50000;
const APP_PATHS: &str = r"Software\Microsoft\Windows\CurrentVersion\App Paths";
const CLSID: &str = r"Software\Classes\CLSID";
const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
fn direct_child<'a>(key: &'a str, parent: &str) -> Option<&'a str> {
    key.strip_prefix(parent)?
        .strip_prefix('\\')
        .filter(|s| !s.is_empty() && !s.contains('\\'))
}
fn issue_category(value: &RegistryValue) -> Option<&'static str> {
    if value.name.is_empty()
        && direct_child(&value.key, APP_PATHS)
            .is_some_and(|s| s.to_ascii_lowercase().ends_with(".exe"))
    {
        return Some("app_paths");
    }
    if value.name == "DisplayIcon" && direct_child(&value.key, UNINSTALL).is_some() {
        return Some("icons");
    }
    if value.name.is_empty() {
        for suffix in ["InprocServer32", "LocalServer32"] {
            if let Some(parent) = value.key.strip_suffix(&format!("\\{suffix}")) {
                if direct_child(parent, CLSID)
                    .is_some_and(|s| uuid::Uuid::parse_str(s.trim_matches(['{', '}'])).is_ok())
                {
                    return Some(if suffix == "InprocServer32" {
                        "com_dll"
                    } else {
                        "com_exe"
                    });
                }
            }
        }
    }
    None
}
fn key_allowed(key: &str) -> bool {
    [RUN, RUN_ONCE].contains(&key)
        || issue_category(&RegistryValue {
            key: key.into(),
            name: String::new(),
            kind: 1,
            data: vec![],
        })
        .is_some()
        || direct_child(key, UNINSTALL).is_some()
}
fn allowed_for(value: &RegistryValue, scope: &str) -> bool {
    value.data.len() <= 1024 * 1024
        && !value.name.contains('\0')
        && value.name.len() <= 32768
        && if scope == "startup" {
            allowed(value)
        } else {
            scope == "registry" && issue_category(value).is_some()
        }
}
fn issue_target(value: &RegistryValue, command: &str) -> Option<String> {
    match issue_category(value)? {
        "com_exe" => executable(command),
        "app_paths" => executable(&format!("\"{}\"", command.trim().trim_matches('"'))),
        category => {
            let text = if category == "icons" {
                command
                    .rsplit_once(',')
                    .filter(|(_, n)| n.trim().parse::<i32>().is_ok())
                    .map(|(s, _)| s)
                    .unwrap_or(command)
            } else {
                command
            };
            let path = text.trim().trim_matches('"');
            // Reuse the strict absolute-path validator, allowing only known resource extensions.
            let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
            if !["dll", "exe", "ico", "ocx"].contains(&extension.as_str()) {
                return None;
            }
            let stem = &path[..path.len() - extension.len()];
            executable(&format!("\"{stem}exe\"")).map(|_| path.to_owned())
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegistryValue {
    pub key: String,
    pub name: String,
    pub kind: u32,
    pub data: Vec<u8>,
}
pub trait Registry: Send {
    fn values(&self) -> Result<Vec<RegistryValue>>;
    fn issue_values(&self) -> Result<Vec<RegistryValue>> {
        Ok(Vec::new())
    }
    fn read(&self, key: &str, name: &str) -> Result<Option<RegistryValue>>;
    fn remove(&mut self, value: &RegistryValue) -> Result<()>;
    /// Must refuse to overwrite any existing value.
    fn restore(&mut self, value: &RegistryValue) -> Result<()>;
}
#[derive(Clone, Serialize)]
struct Finding {
    id: String,
    key: String,
    name: String,
    command: String,
    target: Option<String>,
    state: String,
    removable: bool,
    category: String,
}
struct Snapshot {
    created: Instant,
    values: HashMap<String, (RegistryValue, Finding)>,
}
struct Inner {
    db: Connection,
    registry: Box<dyn Registry>,
    snapshots: HashMap<String, Snapshot>,
}
pub struct Maintenance {
    inner: Mutex<Inner>,
}

// Parse only unambiguous absolute local .exe paths. Shells, environment expansion,
// UNC paths and unquoted paths containing spaces are deliberately review-only.
pub fn executable(command: &str) -> Option<String> {
    let text = command.trim();
    let path = if let Some(quoted) = text.strip_prefix('"') {
        let (path, rest) = quoted.split_once('"')?;
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            return None;
        }
        path
    } else {
        text.split_whitespace().next()?
    };
    let b = path.as_bytes();
    if b.len() < 7
        || !b[0].is_ascii_alphabetic()
        || b[1] != b':'
        || b[2] != b'\\'
        || !path.to_ascii_lowercase().ends_with(".exe")
        || path
            .chars()
            .any(|c| c.is_control() || matches!(c, '%' | '"' | '*' | '?' | '/' | '|' | '<' | '>'))
        || path[2..].contains(':')
        || path
            .split('\\')
            .any(|s| s == ".." || s == "." || s.ends_with(' ') || s.ends_with('.'))
    {
        return None;
    }
    // Launcher existence says nothing about the existence of the launched resource.
    let filename = path.rsplit('\\').next()?.to_ascii_lowercase();
    if [
        "cmd.exe",
        "powershell.exe",
        "pwsh.exe",
        "rundll32.exe",
        "regsvr32.exe",
        "wscript.exe",
        "cscript.exe",
        "mshta.exe",
    ]
    .contains(&filename.as_str())
    {
        return None;
    }
    Some(path.into())
}
fn decode(value: &RegistryValue) -> Option<String> {
    if ![1, 2].contains(&value.kind) || value.data.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = value
        .data
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let text = String::from_utf16(&units).ok()?;
    let text = text.trim_end_matches('\0');
    if text.contains('\0') {
        None
    } else {
        Some(text.into())
    }
}
fn allowed(value: &RegistryValue) -> bool {
    [RUN, RUN_ONCE].contains(&value.key.as_str())
        && !value.name.is_empty()
        && !value.name.contains('\0')
        && value.name.len() <= 32768
        && value.data.len() <= 1024 * 1024
}
fn missing(target: &str) -> bool {
    // Only a definite NotFound counts; denied access and disconnected volumes are not evidence.
    let drive = &target[..3];
    Path::new(drive).is_dir()
        && matches!(std::fs::metadata(target), Err(e) if e.kind()==std::io::ErrorKind::NotFound)
}
impl Maintenance {
    pub fn open(data: &Path) -> Result<Self> {
        Self::with_registry(data, Box::new(NativeRegistry))
    }
    pub fn with_registry(data: &Path, registry: Box<dyn Registry>) -> Result<Self> {
        std::fs::create_dir_all(data)?;
        let db = Connection::open(data.join("maintenance.sqlite3"))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS history(id INTEGER PRIMARY KEY, created TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, kind TEXT NOT NULL, result TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS backups(id TEXT PRIMARY KEY, created TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
              value TEXT NOT NULL, state TEXT NOT NULL, action TEXT NOT NULL);")?;
        Ok(Self {
            inner: Mutex::new(Inner {
                db,
                registry,
                snapshots: HashMap::new(),
            }),
        })
    }
    pub fn record(&self, kind: &str, result: &Value) -> Result<()> {
        self.inner.lock().db.execute(
            "INSERT INTO history(kind,result) VALUES(?,?)",
            params![kind, serde_json::to_string(result)?],
        )?;
        Ok(())
    }
    pub fn history(&self) -> Result<Value> {
        let inner = self.inner.lock();
        let mut q = inner
            .db
            .prepare("SELECT id,created,kind,result FROM history ORDER BY id DESC LIMIT 200")?;
        let rows = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut items = Vec::new();
        for (id, created, kind, result) in rows {
            items.push(json!({"id":id,"created":created,"kind":kind,"result":serde_json::from_str::<Value>(&result)?}));
        }
        Ok(json!({"items":items}))
    }
    pub fn scan(&self) -> Result<Value> {
        self.scan_scope("startup")
    }
    pub fn scan_scope(&self, scope: &str) -> Result<Value> {
        if !["startup", "registry"].contains(&scope) {
            bail!("Неизвестный раздел");
        }
        let mut inner = self.inner.lock();
        let mut findings = Vec::new();
        let mut values = HashMap::new();
        let entries = if scope == "startup" {
            inner.registry.values()?
        } else {
            inner.registry.issue_values()?
        };
        for value in entries {
            if !allowed_for(&value, scope) {
                continue;
            }
            let command =
                decode(&value).unwrap_or_else(|| "Нестроковое значение — только просмотр".into());
            let target = decode(&value).and_then(|s| {
                if scope == "startup" {
                    executable(&s)
                } else {
                    issue_target(&value, &s)
                }
            });
            let absent = target.as_deref().is_some_and(missing);
            let state = if absent {
                "missing"
            } else if target.is_some() {
                "present"
            } else {
                "review"
            };
            let finding = Finding {
                id: uuid::Uuid::new_v4().to_string(),
                key: format!("HKCU\\{}", value.key),
                name: value.name.clone(),
                command,
                target,
                state: state.into(),
                removable: absent,
                category: if scope == "startup" {
                    "startup".into()
                } else {
                    issue_category(&value).unwrap_or("review").into()
                },
            };
            values.insert(finding.id.clone(), (value, finding.clone()));
            findings.push(finding);
        }
        findings.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        inner.snapshots.insert(
            scope.into(),
            Snapshot {
                created: Instant::now(),
                values,
            },
        );
        Ok(
            json!({"items":findings,"supported":cfg!(windows),"scope":if scope == "startup" { "HKCU Run / RunOnce · 64-bit" } else { "HKCU App Paths / COM / DisplayIcon · 64-bit" },"limit":LIMIT,"expires_seconds":900}),
        )
    }
    pub fn backups(&self) -> Result<Value> {
        self.backups_page_size(0, 500)
    }
    pub fn backups_page(&self, offset: i64) -> Result<Value> {
        self.backups_page_size(offset.max(0), 60)
    }
    fn backups_page_size(&self, offset: i64, page_size: i64) -> Result<Value> {
        let inner = self.inner.lock();
        let total: i64 = inner
            .db
            .query_row("SELECT COUNT(*) FROM backups", [], |r| r.get(0))?;
        let offset = offset.min(((total - 1).max(0) / page_size) * page_size);
        let mut q = inner.db.prepare("SELECT id,created,value,state,action FROM backups ORDER BY rowid DESC LIMIT ? OFFSET ?")?;
        let rows = q
            .query_map(params![page_size, offset], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut items = Vec::new();
        for (id, created, value, state, action) in rows {
            let value: RegistryValue = serde_json::from_str(&value)?;
            items.push(json!({"id":id,"created":created,"key":format!("HKCU\\{}",value.key),"name":value.name,"command":decode(&value),"state":state,"action":action}));
        }
        Ok(json!({"items":items,"total":total,"offset":offset,"page_size":page_size}))
    }
    pub fn apply(&self, ids: &[String], action: &str) -> Result<Value> {
        self.apply_scope("startup", ids, action)
    }
    pub fn apply_scope(&self, scope: &str, ids: &[String], action: &str) -> Result<Value> {
        if !["startup", "registry"].contains(&scope) || (scope == "registry" && action != "clean") {
            bail!("Недопустимая операция для раздела");
        }
        if ids.is_empty() || ids.len() > LIMIT {
            bail!("Выберите от 1 до 50 000 записей");
        }
        if !["clean", "disable"].contains(&action) {
            bail!("Неизвестная операция");
        }
        let mut inner = self.inner.lock();
        let snapshot = inner
            .snapshots
            .get(scope)
            .context("Сначала выполните проверку")?;
        if snapshot.created.elapsed().as_secs() > 900 {
            bail!("Проверка устарела. Обновите список");
        }
        if ids.iter().collect::<std::collections::HashSet<_>>().len() != ids.len() {
            bail!("Повторяющиеся идентификаторы");
        }
        let selected = ids
            .iter()
            .map(|id| {
                snapshot
                    .values
                    .get(id)
                    .cloned()
                    .context("Неизвестная запись. Обновите список")
            })
            .collect::<Result<Vec<_>>>()?;
        // Validate every requested id before starting the batch. Never trust client paths.
        for (value, finding) in &selected {
            if !allowed_for(value, scope) || decode(value).is_none() {
                bail!("Эту запись нельзя изменять");
            }
            if action == "clean"
                && (!finding.removable || !finding.target.as_deref().is_some_and(missing))
            {
                bail!("Путь изменился или запись требует ручной проверки");
            }
        }
        let mut done = 0;
        let mut errors = Vec::new();
        for (value, finding) in selected {
            let result = (|| -> Result<()> {
                if inner.registry.read(&value.key, &value.name)?.as_ref() != Some(&value) {
                    bail!("Запись изменилась после проверки");
                }
                let id = uuid::Uuid::new_v4().to_string();
                // Commit complete typed bytes BEFORE touching the registry. Pending records
                // remain restorable after a crash or a failed journal update.
                inner.db.execute(
                    "INSERT INTO backups(id,value,state,action) VALUES(?,?,'pending',?)",
                    params![id, serde_json::to_string(&value)?, action],
                )?;
                inner.registry.remove(&value)?;
                inner
                    .db
                    .execute("UPDATE backups SET state='removed' WHERE id=?", [id])?;
                done += 1;
                Ok(())
            })();
            if let Err(e) = result {
                errors.push(json!({"id":finding.id,"name":value.name,"message":e.to_string()}));
            }
        }
        inner.snapshots.remove(scope);
        Ok(json!({"done":done,"errors":errors}))
    }
    pub fn restore(&self, id: &str) -> Result<Value> {
        let mut inner = self.inner.lock();
        let (raw, state): (String, String) = inner
            .db
            .query_row("SELECT value,state FROM backups WHERE id=?", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .context("Резервная копия не найдена")?;
        let value: RegistryValue = serde_json::from_str(&raw)?;
        if !(allowed(&value) || allowed_for(&value, "registry"))
            || !["pending", "removed"].contains(&state.as_str())
        {
            bail!("Копия недоступна для восстановления");
        }
        match inner.registry.read(&value.key, &value.name)? {
            Some(existing) if existing == value => {} // Idempotent recovery after a crash.
            Some(_) => bail!("В реестре уже есть другая запись. Перезапись запрещена"),
            None => inner.registry.restore(&value)?,
        }
        inner
            .db
            .execute("UPDATE backups SET state='restored' WHERE id=?", [id])?;
        inner.snapshots.clear();
        Ok(json!({"restored":true}))
    }
}
pub struct NativeRegistry;
#[cfg(not(windows))]
impl Registry for NativeRegistry {
    fn values(&self) -> Result<Vec<RegistryValue>> {
        bail!("Реестр доступен только на Windows")
    }
    fn read(&self, _: &str, _: &str) -> Result<Option<RegistryValue>> {
        bail!("Только Windows")
    }
    fn remove(&mut self, _: &RegistryValue) -> Result<()> {
        bail!("Только Windows")
    }
    fn restore(&mut self, _: &RegistryValue) -> Result<()> {
        bail!("Только Windows")
    }
}
#[cfg(windows)]
mod native {
    use super::*;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS},
        System::Registry::*,
    };
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    fn check(code: u32) -> Result<()> {
        if code == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(std::io::Error::from_raw_os_error(code as i32).into())
        }
    }
    fn open(path: &str, write: bool) -> Result<Option<Key>> {
        if !key_allowed(path) {
            bail!("Раздел реестра запрещён");
        }
        let mut key = null_mut();
        let code = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                wide(path).as_ptr(),
                0,
                KEY_QUERY_VALUE | KEY_WOW64_64KEY | if write { KEY_SET_VALUE } else { 0 },
                &mut key,
            )
        };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(code)?;
        Ok(Some(Key(key)))
    }
    fn read_value(key: &Key, path: &str, name: &str) -> Result<Option<RegistryValue>> {
        let mut kind = 0;
        let mut size = 0;
        let code = unsafe {
            RegQueryValueExW(
                key.0,
                wide(name).as_ptr(),
                null(),
                &mut kind,
                null_mut(),
                &mut size,
            )
        };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(code)?;
        if size > 1024 * 1024 {
            bail!("Слишком большое значение");
        }
        let mut data = vec![0; size as usize];
        check(unsafe {
            RegQueryValueExW(
                key.0,
                wide(name).as_ptr(),
                null(),
                &mut kind,
                data.as_mut_ptr(),
                &mut size,
            )
        })?;
        data.truncate(size as usize);
        Ok(Some(RegistryValue {
            key: path.into(),
            name: name.into(),
            kind,
            data,
        }))
    }
    fn children(path: &str) -> Result<Vec<String>> {
        let mut handle = null_mut();
        let code = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                wide(path).as_ptr(),
                0,
                KEY_ENUMERATE_SUB_KEYS | KEY_WOW64_64KEY,
                &mut handle,
            )
        };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(vec![]);
        }
        check(code)?;
        let key = Key(handle);
        let mut names = Vec::new();
        for index in 0..10000 {
            let mut name = vec![0u16; 256];
            let mut length = name.len() as u32;
            let code = unsafe {
                RegEnumKeyExW(
                    key.0,
                    index,
                    name.as_mut_ptr(),
                    &mut length,
                    null(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                )
            };
            if code == ERROR_NO_MORE_ITEMS {
                return Ok(names);
            }
            check(code)?;
            names.push(String::from_utf16(&name[..length as usize])?);
        }
        bail!("Слишком много подразделов. Сканирование не завершено")
    }
    impl Registry for NativeRegistry {
        fn values(&self) -> Result<Vec<RegistryValue>> {
            let mut values = Vec::new();
            for path in [RUN, RUN_ONCE] {
                let Some(key) = open(path, false)? else {
                    continue;
                };
                for index in 0..10000 {
                    let mut name = vec![0u16; 32768];
                    let mut length = name.len() as u32;
                    let code = unsafe {
                        RegEnumValueW(
                            key.0,
                            index,
                            name.as_mut_ptr(),
                            &mut length,
                            null(),
                            null_mut(),
                            null_mut(),
                            null_mut(),
                        )
                    };
                    if code == ERROR_NO_MORE_ITEMS {
                        break;
                    }
                    check(code)?;
                    let name = String::from_utf16(&name[..length as usize])?;
                    if let Some(value) = read_value(&key, path, &name)? {
                        values.push(value);
                    }
                    if index == 9999 {
                        bail!("Слишком много записей автозагрузки");
                    }
                }
            }
            Ok(values)
        }
        fn issue_values(&self) -> Result<Vec<RegistryValue>> {
            let mut values = Vec::new();
            for (root, suffixes, name) in [
                (APP_PATHS, vec![""], ""),
                (CLSID, vec!["InprocServer32", "LocalServer32"], ""),
                (UNINSTALL, vec![""], "DisplayIcon"),
            ] {
                for child in children(root)? {
                    for suffix in &suffixes {
                        let path = if suffix.is_empty() {
                            format!("{root}\\{child}")
                        } else {
                            format!("{root}\\{child}\\{suffix}")
                        };
                        if !key_allowed(&path) {
                            continue;
                        }
                        if let Some(key) = open(&path, false)? {
                            if let Some(value) = read_value(&key, &path, name)? {
                                values.push(value);
                            }
                        }
                    }
                }
            }
            Ok(values)
        }
        fn read(&self, path: &str, name: &str) -> Result<Option<RegistryValue>> {
            match open(path, false)? {
                Some(key) => read_value(&key, path, name),
                None => Ok(None),
            }
        }
        fn remove(&mut self, value: &RegistryValue) -> Result<()> {
            let key = open(&value.key, true)?.context("Раздел исчез")?;
            if read_value(&key, &value.key, &value.name)?.as_ref() != Some(value) {
                bail!("Запись изменилась");
            }
            check(unsafe { RegDeleteValueW(key.0, wide(&value.name).as_ptr()) })
        }
        fn restore(&mut self, value: &RegistryValue) -> Result<()> {
            let key = open(&value.key, true)?
                .context("Раздел исчез. Автоматическое создание запрещено")?;
            if read_value(&key, &value.key, &value.name)?.is_some() {
                bail!("Запись уже существует");
            }
            check(unsafe {
                RegSetValueExW(
                    key.0,
                    wide(&value.name).as_ptr(),
                    0,
                    value.kind,
                    value.data.as_ptr(),
                    value.data.len() as u32,
                )
            })
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    #[derive(Clone, Default)]
    struct Fake {
        entries: Arc<Mutex<Vec<RegistryValue>>>,
        fail_remove: bool,
    }
    impl Registry for Fake {
        fn values(&self) -> Result<Vec<RegistryValue>> {
            Ok(self.entries.lock().clone())
        }
        fn issue_values(&self) -> Result<Vec<RegistryValue>> {
            Ok(self.entries.lock().clone())
        }
        fn read(&self, k: &str, n: &str) -> Result<Option<RegistryValue>> {
            Ok(self
                .entries
                .lock()
                .iter()
                .find(|v| v.key == k && v.name == n)
                .cloned())
        }
        fn remove(&mut self, v: &RegistryValue) -> Result<()> {
            if self.fail_remove {
                bail!("simulated failure");
            }
            self.entries.lock().retain(|x| x != v);
            Ok(())
        }
        fn restore(&mut self, v: &RegistryValue) -> Result<()> {
            if self.read(&v.key, &v.name)?.is_some() {
                bail!("conflict");
            }
            self.entries.lock().push(v.clone());
            Ok(())
        }
    }
    fn value() -> RegistryValue {
        RegistryValue { key:RUN.into(),name:"Test app".into(),kind:2,data:r#""C:\Apps\example.exe" --tray"#.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect() }
    }
    fn id(service: &Maintenance) -> String {
        service.scan().unwrap()["items"][0]["id"]
            .as_str()
            .unwrap()
            .into()
    }
    #[test]
    fn bulk_startup_selection_and_scopes_do_not_interfere() {
        let t = tempfile::tempdir().unwrap();
        let fake = Fake::default();
        for i in 0..175 {
            let mut v = value();
            v.name = format!("own-{i}");
            fake.entries.lock().push(v);
        }
        let m = Maintenance::with_registry(t.path(), Box::new(fake.clone())).unwrap();
        let scan = m.scan().unwrap();
        let ids = scan["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert!(
            m.scan_scope("registry").unwrap()["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(m.apply_scope("registry", &ids, "clean").is_err());
        assert!(
            m.apply(&[ids[0].clone(), ids[0].clone()], "disable")
                .is_err()
        );
        assert_eq!(fake.entries.lock().len(), 175);
        assert_eq!(m.apply(&ids, "disable").unwrap()["done"], 175);
        assert_eq!(
            m.backups_page(120).unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            55
        );
        assert_eq!(m.backups_page(120).unwrap()["total"], 175);
        let backups = m.backups().unwrap();
        for b in backups["items"].as_array().unwrap() {
            m.restore(b["id"].as_str().unwrap()).unwrap();
        }
        assert_eq!(fake.entries.lock().len(), 175);
    }
    #[test]
    fn registry_rules_validate_resource_paths_and_do_not_accept_startup_or_trees() {
        let mut v = value();
        v.key = format!("{APP_PATHS}\\fixture.exe");
        v.name.clear();
        assert_eq!(issue_category(&v), Some("app_paths"));
        assert_eq!(
            issue_target(&v, r"C:\Program Files\fixture.exe").as_deref(),
            Some(r"C:\Program Files\fixture.exe")
        );
        v.key = format!("{CLSID}\\{{12345678-1234-1234-1234-123456789012}}\\InprocServer32");
        assert_eq!(
            issue_target(&v, r"C:\QA\fixture.dll").as_deref(),
            Some(r"C:\QA\fixture.dll")
        );
        for command in [
            r"%SystemRoot%\fixture.dll",
            r"\\server\fixture.dll",
            r"C:\QA\..\fixture.dll",
            r"relative.dll",
            r"C:\QA\fixture.dll:stream",
        ] {
            assert!(issue_target(&v, command).is_none());
        }
        v.key = format!("{UNINSTALL}\\fixture");
        v.name = "DisplayIcon".into();
        assert_eq!(
            issue_target(&v, r#""C:\QA\fixture.dll",-1"#).as_deref(),
            Some(r"C:\QA\fixture.dll")
        );
        v.name = "UninstallString".into();
        assert!(issue_category(&v).is_none());
        assert!(issue_category(&value()).is_none());
        assert!(!key_allowed(r"Software\Classes"));
    }
    #[cfg(windows)]
    #[test]
    fn registry_issue_cleanup_revalidates_and_restores_default_value() {
        let t = tempfile::tempdir().unwrap();
        let fake = Fake::default();
        let target = t
            .path()
            .join("own-missing.exe")
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_owned();
        let v = RegistryValue {
            key: format!("{APP_PATHS}\\OwnFixture.exe"),
            name: String::new(),
            kind: 1,
            data: target
                .encode_utf16()
                .chain(Some(0))
                .flat_map(u16::to_le_bytes)
                .collect(),
        };
        fake.entries.lock().push(v.clone());
        let m = Maintenance::with_registry(t.path(), Box::new(fake.clone())).unwrap();
        let find = m.scan_scope("registry").unwrap();
        let id = find["items"][0]["id"].as_str().unwrap().to_owned();
        assert_eq!(find["items"][0]["removable"], true);
        std::fs::write(&target, b"own file appeared after preview").unwrap();
        assert!(m.apply_scope("registry", &[id.clone()], "clean").is_err());
        assert_eq!(fake.entries.lock()[0], v);
        std::fs::remove_file(&target).unwrap();
        assert_eq!(
            m.apply_scope("registry", &[id], "clean").unwrap()["done"],
            1
        );
        let b = m.backups().unwrap();
        m.restore(b["items"][0]["id"].as_str().unwrap()).unwrap();
        assert_eq!(fake.entries.lock()[0], v);
    }
    #[test]
    fn command_parser_is_conservative() {
        assert_eq!(
            executable(r#""C:\Program Files\App\app.exe" --tray"#).as_deref(),
            Some(r"C:\Program Files\App\app.exe")
        );
        assert_eq!(
            executable(r"D:\app.exe /background").as_deref(),
            Some(r"D:\app.exe")
        );
        for cmd in [
            r"C:\Program Files\App\app.exe",
            r"%APPDATA%\app.exe",
            r"\\server\app.exe",
            r"app.exe",
            r"C:\Windows\System32\rundll32.exe x.dll",
            r"C:\a\..\app.exe",
            r#""C:\app.exe"bad"#,
            r"C:\app.exe:stream",
            r"C:\app?.exe",
        ] {
            assert!(executable(cmd).is_none(), "{cmd}");
        }
    }
    #[test]
    fn disable_has_durable_typed_backup_and_restores_exact_bytes() {
        let t = tempfile::tempdir().unwrap();
        let fake = Fake::default();
        fake.entries.lock().push(value());
        let m = Maintenance::with_registry(t.path(), Box::new(fake.clone())).unwrap();
        let id = id(&m);
        assert_eq!(m.apply(&[id], "disable").unwrap()["done"], 1);
        assert!(fake.entries.lock().is_empty());
        drop(m);
        let m = Maintenance::with_registry(t.path(), Box::new(fake.clone())).unwrap();
        let b = m.backups().unwrap()["items"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(m.restore(&b).unwrap()["restored"], true);
        assert_eq!(fake.entries.lock()[0], value());
        assert!(m.restore(&b).is_err());
    }
    #[test]
    fn changed_values_unknown_ids_and_unsupported_keys_are_rejected() {
        let t = tempfile::tempdir().unwrap();
        let fake = Fake::default();
        fake.entries.lock().push(value());
        let m = Maintenance::with_registry(t.path(), Box::new(fake.clone())).unwrap();
        let id = id(&m);
        assert!(
            m.apply(&[id.clone(), "invented".into()], "disable")
                .is_err()
        );
        assert_eq!(fake.entries.lock().len(), 1);
        fake.entries.lock()[0].data.push(0);
        assert_eq!(m.apply(&[id], "disable").unwrap()["done"], 0);
        assert!(m.backups().unwrap()["items"].as_array().unwrap().is_empty());
        let mut bad = value();
        bad.key = r"Software\Classes".into();
        fake.entries.lock().clear();
        fake.entries.lock().push(bad);
        assert!(m.scan().unwrap()["items"].as_array().unwrap().is_empty());
    }
    #[test]
    fn restore_refuses_replacement_and_pending_failure_remains_recoverable() {
        let t = tempfile::tempdir().unwrap();
        let fake = Fake {
            fail_remove: true,
            ..Default::default()
        };
        fake.entries.lock().push(value());
        let m = Maintenance::with_registry(t.path(), Box::new(fake.clone())).unwrap();
        let id = id(&m);
        assert_eq!(
            m.apply(&[id], "disable").unwrap()["errors"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let backups = m.backups().unwrap();
        let b = backups["items"][0]["id"].as_str().unwrap();
        assert_eq!(backups["items"][0]["state"], "pending");
        fake.entries.lock()[0].kind = 1;
        assert!(m.restore(b).is_err());
        assert_eq!(fake.entries.lock()[0].kind, 1);
        fake.entries.lock().clear();
        assert!(m.restore(b).is_ok());
        assert_eq!(fake.entries.lock()[0], value());
    }
    #[test]
    fn cleanup_never_removes_present_or_ambiguous_commands() {
        let t = tempfile::tempdir().unwrap();
        let fake = Fake::default();
        let mut v = value();
        v.data = "relative.exe"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        fake.entries.lock().push(v);
        let m = Maintenance::with_registry(t.path(), Box::new(fake.clone())).unwrap();
        let id = id(&m);
        assert!(m.apply(&[id], "clean").is_err());
        assert_eq!(fake.entries.lock().len(), 1);
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;
    #[test]
    fn history_survives_restart_and_preserves_partial_error_details() {
        let t = tempfile::tempdir().unwrap();
        let m = Maintenance::open(t.path()).unwrap();
        let result = json!({"done":1,"moved_bytes":42,"freed_bytes":0,"errors":[{"id":7,"message":"changed"}],"elapsed_ms":99});
        m.record("/api/space/trash", &result).unwrap();
        drop(m);
        let reopened = Maintenance::open(t.path()).unwrap();
        let history = reopened.history().unwrap();
        assert_eq!(history["items"][0]["result"], result);
        assert_eq!(history["items"][0]["kind"], "/api/space/trash");
    }
}
#[cfg(all(test, windows))]
mod native_qa {
    use super::*;
    #[test]
    #[ignore = "Explicit native QA: creates and removes only its own UUID-named startup value"]
    fn native_registry_clean_restore_own_value() {
        let base =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".cleaner_ultra/native-qa");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("registry-")
            .tempdir_in(&base)
            .unwrap();
        let name = format!("CleanerUltra.QA.{}", uuid::Uuid::new_v4());
        let command = format!(
            "\"{}\" --qa",
            dir.path()
                .join("intentionally-absent.exe")
                .to_string_lossy()
                .replace('/', "\\")
        );
        let value = RegistryValue {
            key: RUN.into(),
            name: name.clone(),
            kind: 1,
            data: command
                .encode_utf16()
                .chain(Some(0))
                .flat_map(u16::to_le_bytes)
                .collect(),
        };
        let mut registry = NativeRegistry;
        assert!(registry.read(RUN, &name).unwrap().is_none());
        registry.restore(&value).unwrap();
        struct Cleanup(RegistryValue);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let mut r = NativeRegistry;
                if r.read(&self.0.key, &self.0.name).ok().flatten().as_ref() == Some(&self.0) {
                    let _ = r.remove(&self.0);
                }
            }
        }
        let _cleanup = Cleanup(value.clone());
        let service = Maintenance::open(dir.path()).unwrap();
        let scan = service.scan().unwrap();
        let id = scan["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == name)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(service.apply(&[id], "clean").unwrap()["done"], 1);
        assert!(registry.read(RUN, &name).unwrap().is_none());
        let b = service.backups().unwrap()["items"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        service.restore(&b).unwrap();
        assert_eq!(registry.read(RUN, &name).unwrap(), Some(value));
        println!(
            "native registry cleanup + exact restore verified; own UUID value removed by guard"
        );
    }
}
