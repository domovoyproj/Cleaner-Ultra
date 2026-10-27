//! Explicit, recoverable maintenance of the current user's startup registry values.
//! No HKLM, services, COM, uninstall keys or registry trees are ever deleted.
use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, path::Path, time::Instant};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_ONCE: &str = r"Software\Microsoft\Windows\CurrentVersion\RunOnce";
const LIMIT: usize = 100;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegistryValue {
    pub key: String,
    pub name: String,
    pub kind: u32,
    pub data: Vec<u8>,
}
pub trait Registry: Send {
    fn values(&self) -> Result<Vec<RegistryValue>>;
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
}
struct Snapshot {
    created: Instant,
    values: HashMap<String, (RegistryValue, Finding)>,
}
struct Inner {
    db: Connection,
    registry: Box<dyn Registry>,
    snapshot: Option<Snapshot>,
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
                snapshot: None,
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
        let mut inner = self.inner.lock();
        let mut findings = Vec::new();
        let mut values = HashMap::new();
        for value in inner.registry.values()? {
            if !allowed(&value) {
                continue;
            }
            let command =
                decode(&value).unwrap_or_else(|| "Нестроковое значение — только просмотр".into());
            let target = decode(&value).and_then(|s| executable(&s));
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
            };
            values.insert(finding.id.clone(), (value, finding.clone()));
            findings.push(finding);
        }
        findings.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        inner.snapshot = Some(Snapshot {
            created: Instant::now(),
            values,
        });
        Ok(
            json!({"items":findings,"supported":cfg!(windows),"scope":"HKCU Run / RunOnce · 64-bit","expires_seconds":900}),
        )
    }
    pub fn backups(&self) -> Result<Value> {
        let inner = self.inner.lock();
        let mut q = inner.db.prepare(
            "SELECT id,created,value,state,action FROM backups ORDER BY rowid DESC LIMIT 500",
        )?;
        let rows = q
            .query_map([], |r| {
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
        Ok(json!({"items":items}))
    }
    pub fn apply(&self, ids: &[String], action: &str) -> Result<Value> {
        if ids.is_empty() || ids.len() > LIMIT {
            bail!("Выберите от 1 до 100 записей");
        }
        if !["clean", "disable"].contains(&action) {
            bail!("Неизвестная операция");
        }
        let mut inner = self.inner.lock();
        let snapshot = inner
            .snapshot
            .as_ref()
            .context("Сначала выполните проверку")?;
        if snapshot.created.elapsed().as_secs() > 900 {
            bail!("Проверка устарела. Обновите список");
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
            if !allowed(value) || decode(value).is_none() {
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
        for (value, _) in selected {
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
                errors.push(json!({"name":value.name,"message":e.to_string()}));
            }
        }
        inner.snapshot = None;
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
        if !allowed(&value) || !["pending", "removed"].contains(&state.as_str()) {
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
        inner.snapshot = None;
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
        if ![RUN, RUN_ONCE].contains(&path) {
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
