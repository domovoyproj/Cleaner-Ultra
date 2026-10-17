use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

const PAGE: i64 = 60;
const VAULT: &str = ".photosort-space-trash";

#[derive(Clone, Default, Serialize)]
pub struct ScanProgress {
    pub id: i64,
    pub root: String,
    pub state: String,
    pub files: u64,
    pub bytes: u64,
    pub errors: u64,
    pub current: String,
    pub started: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SpaceFile {
    pub id: i64,
    pub path: String,
    pub size: u64,
    pub modified: u64,
    pub category: String,
    pub temporary: bool,
    pub protected: bool,
    pub stamp: u64,
}

pub struct Space {
    db: Mutex<Connection>,
    pub progress: Mutex<ScanProgress>,
    cancel: AtomicBool,
    mutation: Mutex<()>,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
    data: PathBuf,
    protected_roots: Vec<String>,
}

fn normal(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(p) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{p}"));
        }
        if let Some(p) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(p);
        }
    }
    PathBuf::from(s.as_ref())
}
fn canonical(path: &Path) -> Result<PathBuf> {
    Ok(normal(&fs::canonicalize(path)?))
}
fn path_key(path: &Path) -> String {
    let key = normal(path).to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    let key = key.to_lowercase();
    key.trim_end_matches('/').to_string()
}
fn excluded(path: &Path, keys: &[String]) -> bool {
    if keys.is_empty() {
        return false;
    }
    let path = path_key(path);
    keys.binary_search(&path).is_ok()
        || path.match_indices('/').any(|(i, _)| {
            keys.binary_search_by(|key| key.as_str().cmp(&path[..i]))
                .is_ok()
        })
}
fn exclusion_path(value: &str) -> Result<PathBuf> {
    let path = normal(Path::new(value.trim()));
    if !path.is_absolute() || value.len() > 32768 || value.contains('\0') {
        bail!("Укажите полный путь к папке или файлу")
    }
    let mut clean = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                clean.pop();
            }
            std::path::Component::CurDir => {}
            other => clean.push(other.as_os_str()),
        }
    }
    // Resolve existing ancestors too: a temporarily absent file may live under a junction.
    let mut ancestor = clean.as_path();
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        let Some(name) = ancestor.file_name() else {
            break;
        };
        suffix.push(name.to_owned());
        ancestor = ancestor.parent().context("Некорректный путь")?;
    }
    let mut resolved = if ancestor.exists() {
        canonical(ancestor)?
    } else {
        ancestor.to_path_buf()
    };
    for name in suffix.into_iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}
fn timestamp(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().min(i64::MAX as u128) as u64)
        .unwrap_or(0)
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn category(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" | "png" | "webp" | "heic" | "heif" | "avif" | "tif" | "tiff" | "dng"
        | "cr2" | "cr3" | "nef" | "arw" | "gif" | "bmp" | "raw" | "rw2" | "orf" | "raf" | "pef"
        | "psd" | "svg" | "ico" => "photos",
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" | "wmv" | "mts" | "m2ts" | "mpg"
        | "mpeg" => "video",
        "mp3" | "flac" | "wav" | "aac" | "ogg" | "m4a" => "audio",
        "zip" | "7z" | "rar" | "tar" | "gz" | "iso" | "vhd" | "vhdx" => "archives",
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "md" | "csv" => {
            "documents"
        }
        "exe" | "msi" | "msix" | "dll" | "sys" => "apps",
        _ => "other",
    }
}
fn row(r: &rusqlite::Row) -> rusqlite::Result<SpaceFile> {
    Ok(SpaceFile {
        id: r.get(0)?,
        path: r.get(1)?,
        size: r.get(2)?,
        modified: r.get(3)?,
        category: r.get(4)?,
        temporary: r.get(5)?,
        protected: r.get(6)?,
        stamp: r.get(7)?,
    })
}
const COLS: &str = "id,path,size,modified,category,temporary,protected,stamp";

impl Space {
    pub fn open(data: &Path) -> Result<Arc<Self>> {
        fs::create_dir_all(data)?;
        let data = canonical(data)?;
        let db = Connection::open(data.join("space.sqlite3"))?;
        db.busy_timeout(std::time::Duration::from_secs(30))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
          CREATE TABLE IF NOT EXISTS scans(id INTEGER PRIMARY KEY,root TEXT,state TEXT,started INTEGER,files INTEGER DEFAULT 0,bytes INTEGER DEFAULT 0,errors INTEGER DEFAULT 0);
          CREATE TABLE IF NOT EXISTS files(id INTEGER PRIMARY KEY,scan INTEGER,path TEXT,name TEXT,size INTEGER,modified INTEGER,category TEXT,temporary INTEGER,protected INTEGER,stamp INTEGER DEFAULT 0,parent TEXT DEFAULT '', UNIQUE(scan,path));
          CREATE INDEX IF NOT EXISTS space_size ON files(scan,size DESC,id);
          CREATE INDEX IF NOT EXISTS space_category ON files(scan,category,size DESC);
          CREATE TABLE IF NOT EXISTS folders(scan INTEGER,path TEXT,parent TEXT,bytes INTEGER,files INTEGER,PRIMARY KEY(scan,path));
          CREATE INDEX IF NOT EXISTS space_parent ON folders(scan,parent,bytes DESC);
          CREATE TABLE IF NOT EXISTS trash(id INTEGER PRIMARY KEY,original TEXT,vault TEXT,root TEXT,size INTEGER,hash TEXT,state TEXT,created INTEGER);
          CREATE TABLE IF NOT EXISTS exclusions(path TEXT PRIMARY KEY);
          CREATE TABLE IF NOT EXISTS space_settings(key TEXT PRIMARY KEY,value INTEGER NOT NULL);
          INSERT OR IGNORE INTO space_settings VALUES('exclusions_revision',0),('scan_exclusions_revision',0);
          UPDATE scans SET state='interrupted' WHERE state='running';")?;
        let columns = db
            .prepare("PRAGMA table_info(files)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !columns.iter().any(|name| name == "stamp") {
            db.execute("ALTER TABLE files ADD COLUMN stamp INTEGER DEFAULT 0", [])?;
        }
        if !columns.iter().any(|name| name == "parent") {
            db.execute("ALTER TABLE files ADD COLUMN parent TEXT DEFAULT ''", [])?;
        }
        // Batches also resume an interrupted upgrade of a large existing index.
        let mut migrated = 0i64;
        loop {
            let records = db
                .prepare(
                    "SELECT id,path FROM files WHERE parent='' AND id>? ORDER BY id LIMIT 512",
                )?
                .query_map([migrated], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if records.is_empty() {
                break;
            }
            let tx = db.unchecked_transaction()?;
            for (id, path) in records {
                let parent = Path::new(&path)
                    .parent()
                    .context("Некорректный путь в индексе")?;
                tx.execute(
                    "UPDATE files SET parent=? WHERE id=?",
                    params![parent.to_string_lossy(), id],
                )?;
                migrated = id;
            }
            tx.commit()?;
        }
        db.execute(
            "CREATE INDEX IF NOT EXISTS space_file_parent ON files(scan,parent,size DESC,id)",
            [],
        )?;
        let progress = db.query_row("SELECT id,root,state,files,bytes,errors,started FROM scans ORDER BY id DESC LIMIT 1", [], |r| Ok(ScanProgress {id:r.get(0)?,root:r.get(1)?,state:r.get(2)?,files:r.get(3)?,bytes:r.get(4)?,errors:r.get(5)?,started:r.get(6)?,current:String::new()})).optional()?.unwrap_or_default();
        let mut protected_roots: Vec<String> = [
            "SystemRoot",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramData",
        ]
        .iter()
        .filter_map(|key| std::env::var_os(key))
        .map(|p| {
            PathBuf::from(p)
                .to_string_lossy()
                .trim_end_matches(['\\', '/'])
                .to_lowercase()
        })
        .collect();
        if let Some(parent) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
        {
            protected_roots.push(normal(&parent).to_string_lossy().to_lowercase());
        }
        let space = Arc::new(Self {
            db: Mutex::new(db),
            progress: Mutex::new(progress),
            cancel: AtomicBool::new(false),
            mutation: Mutex::new(()),
            worker: Mutex::new(None),
            data,
            protected_roots,
        });
        space.recover()?;
        Ok(space)
    }
    fn protected(&self, path: &Path) -> bool {
        if path.starts_with(&self.data)
            || path.ancestors().any(|p| {
                p.file_name().and_then(|s| s.to_str()).is_some_and(|s| {
                    matches!(
                        s.to_lowercase().as_str(),
                        ".photosort-trash"
                            | ".photosort-space-trash"
                            | "$recycle.bin"
                            | "system volume information"
                            | "windows.old"
                            | "$windows.~bt"
                            | "$windows.~ws"
                            | "recovery"
                            | "boot"
                            | "efi"
                    )
                })
            })
        {
            return true;
        }
        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        if matches!(
            filename.as_str(),
            "pagefile.sys"
                | "hiberfil.sys"
                | "swapfile.sys"
                | "bootmgr"
                | "bootnxt"
                | "dumpstack.log.tmp"
        ) || filename.starts_with("ntuser.dat")
            || filename.starts_with("usrclass.dat")
        {
            return true;
        }
        let p = path.to_string_lossy().to_lowercase();
        self.protected_roots.iter().any(|root| {
            p == *root
                || p.strip_prefix(root)
                    .is_some_and(|tail| tail.starts_with(['\\', '/']))
        })
    }
    pub fn exclusions(&self) -> Result<Vec<String>> {
        Ok(self
            .db
            .lock()
            .prepare("SELECT path FROM exclusions ORDER BY path")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn save_exclusions(&self, paths: &[String]) -> Result<Value> {
        let _guard = self.mutation.lock();
        if self.progress.lock().state == "running" {
            bail!("Сначала остановите анализ, чтобы изменить исключения")
        }
        if paths.len() > 256 {
            bail!("Можно добавить не больше 256 исключений")
        }
        let mut unique = std::collections::BTreeMap::new();
        for value in paths {
            let path = exclusion_path(value)?;
            unique
                .entry(path_key(&path))
                .or_insert_with(|| path.to_string_lossy().into_owned());
        }
        let existing = self
            .exclusions()?
            .iter()
            .map(|p| path_key(Path::new(p)))
            .collect::<std::collections::BTreeSet<_>>();
        if existing != unique.keys().cloned().collect() {
            let mut db = self.db.lock();
            let tx = db.transaction()?;
            tx.execute("DELETE FROM exclusions", [])?;
            for path in unique.values() {
                tx.execute("INSERT INTO exclusions(path) VALUES(?)", [path])?;
            }
            tx.execute(
                "UPDATE space_settings SET value=value+1 WHERE key='exclusions_revision'",
                [],
            )?;
            tx.commit()?;
        }
        Ok(json!({"paths":self.exclusions()?}))
    }
    fn exclusions_pending(db: &Connection) -> Result<bool> {
        Ok(db.query_row("SELECT (SELECT value FROM space_settings WHERE key='exclusions_revision')<>(SELECT value FROM space_settings WHERE key='scan_exclusions_revision')", [], |r| r.get(0))?)
    }
    pub fn start(self: &Arc<Self>, path: &str) -> Result<Value> {
        let _guard = self.mutation.lock();
        if self.progress.lock().state == "running" {
            bail!("Анализ уже идёт. Сначала остановите его")
        }
        if let Some(worker) = self.worker.lock().take() {
            let _ = worker.join();
        }
        let root = canonical(Path::new(path)).context("Папка недоступна")?;
        if !root.is_dir() || root.starts_with(&self.data) {
            bail!("Выберите диск или папку с файлами")
        }
        let mut exclusions = self
            .exclusions()?
            .iter()
            .map(|p| path_key(Path::new(p)))
            .collect::<Vec<_>>();
        exclusions.sort_unstable();
        if excluded(&root, &exclusions) {
            bail!("Этот путь входит в исключения. Выберите другую папку или измените исключения")
        }
        let id = {
            let mut db = self.db.lock();
            let tx = db.transaction()?;
            tx.execute("UPDATE space_settings SET value=(SELECT value FROM space_settings WHERE key='exclusions_revision') WHERE key='scan_exclusions_revision'", [])?;
            tx.execute(
                "INSERT INTO scans(root,state,started) VALUES(?,'running',?)",
                params![root.to_string_lossy(), now()],
            )?;
            let id = tx.last_insert_rowid();
            tx.commit()?;
            id
        };
        self.cancel.store(false, Ordering::Relaxed);
        *self.progress.lock() = ScanProgress {
            id,
            root: root.to_string_lossy().into(),
            state: "running".into(),
            started: now(),
            ..Default::default()
        };
        let space = self.clone();
        *self.worker.lock() = Some(thread::spawn(move || {
            if let Err(error) = space.scan(id, &root, &exclusions) {
                space.progress.lock().state = "error".into();
                space.progress.lock().current = error.to_string();
                let _ = space
                    .db
                    .lock()
                    .execute("UPDATE scans SET state='error' WHERE id=?", [id]);
            }
        }));
        Ok(json!({"id":id}))
    }
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn close(&self) {
        self.stop();
        if let Some(worker) = self.worker.lock().take() {
            let _ = worker.join();
        }
    }
    fn scan(&self, id: i64, root: &Path, exclusions: &[String]) -> Result<()> {
        let temp_root = std::env::var_os("TEMP")
            .map(PathBuf::from)
            .and_then(|p| canonical(&p).ok());
        let cutoff = now().saturating_sub(7 * 86400);
        let mut batch = Vec::new();
        let mut files = 0u64;
        let mut bytes = 0u64;
        let mut errors = 0u64;
        let walker = walkdir::WalkDir::new(root)
            .follow_links(false)
            .same_file_system(true)
            .into_iter()
            .filter_entry(|entry| {
                if entry.depth() == 0 {
                    return true;
                }
                let name = entry.file_name().to_string_lossy().to_lowercase();
                !matches!(
                    name.as_str(),
                    ".photosort-trash"
                        | ".photosort-space-trash"
                        | "$recycle.bin"
                        | "system volume information"
                ) && !entry.path().starts_with(&self.data)
                    && !excluded(entry.path(), exclusions)
            });
        for entry in walker {
            if self.cancel.load(Ordering::Relaxed) {
                break;
            }
            let entry = match entry {
                Ok(e) => e,
                Err(_) => {
                    errors += 1;
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => {
                    errors += 1;
                    continue;
                }
            };
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    continue;
                }
            }
            let path = entry.path().to_path_buf();
            let modified = timestamp(&meta);
            let temp = temp_root.as_ref().is_some_and(|p| path.starts_with(p))
                && modified > 0
                && modified / 1_000_000_000 < cutoff;
            files += 1;
            bytes = bytes.saturating_add(meta.len());
            batch.push((path, meta.len(), modified, temp));
            if batch.len() >= 512 {
                self.write_batch(id, root, &batch)?;
                batch.clear();
                self.checkpoint(id, files, bytes, errors, entry.path())?;
            }
        }
        self.write_batch(id, root, &batch)?;
        self.checkpoint(id, files, bytes, errors, root)?;
        let state = if self.cancel.load(Ordering::Relaxed) {
            "cancelled"
        } else {
            "complete"
        };
        self.db
            .lock()
            .execute("UPDATE scans SET state=? WHERE id=?", params![state, id])?;
        self.progress.lock().state = state.into();
        // Only the latest snapshot is browsed; cleanup history is independent.
        let mut db = self.db.lock();
        let tx = db.transaction()?;
        tx.execute("DELETE FROM files WHERE scan<>?", [id])?;
        tx.execute("DELETE FROM folders WHERE scan<>?", [id])?;
        tx.commit()?;
        Ok(())
    }
    fn write_batch(&self, id: i64, root: &Path, batch: &[(PathBuf, u64, u64, bool)]) -> Result<()> {
        let mut db = self.db.lock();
        let tx = db.transaction()?;
        let mut folders = std::collections::HashMap::<PathBuf, (u64, u64)>::new();
        for (path, size, modified, temp) in batch {
            tx.execute("INSERT OR REPLACE INTO files(scan,path,name,size,modified,category,temporary,protected,stamp,parent) VALUES(?,?,?,?,?,?,?,?,?,?)",params![id,path.to_string_lossy(),path.file_name().unwrap_or_default().to_string_lossy().to_lowercase(),size,modified/1_000_000_000,category(path),temp,self.protected(path),modified,path.parent().unwrap_or(root).to_string_lossy()])?;
            for parent in path.ancestors().skip(1).take_while(|p| p.starts_with(root)) {
                let f = folders.entry(parent.to_path_buf()).or_default();
                f.0 = f.0.saturating_add(*size);
                f.1 += 1;
            }
        }
        for (path, (bytes, files)) in folders {
            tx.execute("INSERT INTO folders(scan,path,parent,bytes,files) VALUES(?,?,?,?,?) ON CONFLICT(scan,path) DO UPDATE SET bytes=bytes+excluded.bytes,files=files+excluded.files",params![id,path.to_string_lossy(),path.parent().map(|p|p.to_string_lossy().to_string()).unwrap_or_default(),bytes,files])?;
        }
        tx.commit()?;
        Ok(())
    }
    fn checkpoint(
        &self,
        id: i64,
        files: u64,
        bytes: u64,
        errors: u64,
        current: &Path,
    ) -> Result<()> {
        self.db.lock().execute(
            "UPDATE scans SET files=?,bytes=?,errors=? WHERE id=?",
            params![files, bytes, errors, id],
        )?;
        let mut p = self.progress.lock();
        p.files = files;
        p.bytes = bytes;
        p.errors = errors;
        p.current = current.to_string_lossy().into();
        Ok(())
    }
    pub fn status(&self) -> Result<Value> {
        let p = self.progress.lock().clone();
        let db = self.db.lock();
        let exclusions_pending = p.id != 0 && Self::exclusions_pending(&db)?;
        let exclusion_count: u64 =
            db.query_row("SELECT COUNT(*) FROM exclusions", [], |r| r.get(0))?;
        let categories=db.prepare("SELECT category,SUM(size),COUNT(*) FROM files WHERE scan=? GROUP BY category ORDER BY SUM(size) DESC")?.query_map([p.id],|r|Ok(json!({"category":r.get::<_,String>(0)?,"bytes":r.get::<_,u64>(1)?,"files":r.get::<_,u64>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let cleanup:(u64,u64)=db.query_row("SELECT COALESCE(SUM(size),0),COUNT(*) FROM files WHERE scan=? AND temporary=1 AND protected=0",[p.id],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let large:(u64,u64)=db.query_row("SELECT COALESCE(SUM(size),0),COUNT(*) FROM files WHERE scan=? AND size>=104857600 AND protected=0",[p.id],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let trash: (u64, u64) = db.query_row(
            "SELECT COALESCE(SUM(size),0),COUNT(*) FROM trash WHERE state IN('stored','pending','restoring')",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(
            json!({"progress":p,"exclusions_pending":exclusions_pending,"exclusion_count":exclusion_count,"drives":drives(),"administrator":is_administrator(),"categories":categories,"temporary":{"bytes":cleanup.0,"files":cleanup.1},"large":{"bytes":large.0,"files":large.1},"trash":{"bytes":trash.0,"files":trash.1},"size_kind":"logical"}),
        )
    }
    pub fn files(
        &self,
        mode: &str,
        search: &str,
        offset: i64,
        folder: &str,
        kind: &str,
    ) -> Result<Value> {
        let p = self.progress.lock().clone();
        let db = self.db.lock();
        let condition = match mode {
            "temporary" => "AND temporary=1 AND protected=0",
            "large" => "AND size>=104857600",
            _ => "",
        };
        let all=db.prepare(&format!("SELECT {COLS} FROM files WHERE scan=? {condition} AND instr(name,?)>0 AND (?='' OR category=?) AND (?='' OR path LIKE ? ESCAPE '!') ORDER BY size DESC,id LIMIT ? OFFSET ?"))?.query_map(params![p.id,search.to_lowercase(),kind,kind,folder,folder_pattern(folder),PAGE,offset.max(0)],row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        let total:u64=db.query_row(&format!("SELECT COUNT(*) FROM files WHERE scan=? {condition} AND instr(name,?)>0 AND (?='' OR category=?) AND (?='' OR path LIKE ? ESCAPE '!')"),params![p.id,search.to_lowercase(),kind,kind,folder,folder_pattern(folder)],|r|r.get(0))?;
        Ok(json!({"files":all,"total":total,"offset":offset.max(0),"page_size":PAGE,"scan":p.id}))
    }
    pub fn folders(&self, path: &str, offset: i64) -> Result<Value> {
        let p = self.progress.lock().clone();
        let root = PathBuf::from(&p.root);
        let folder = if path.is_empty() {
            root.clone()
        } else {
            PathBuf::from(path)
        };
        if !folder.starts_with(&root) {
            bail!("Папка вне текущего анализа")
        }
        let db = self.db.lock();
        let total: i64 = db.query_row(
            "SELECT COUNT(*) FROM folders WHERE scan=? AND parent=?",
            params![p.id, folder.to_string_lossy()],
            |r| r.get(0),
        )?;
        let mut entries=db.prepare("SELECT path,bytes,files FROM folders WHERE scan=? AND parent=? ORDER BY bytes DESC,path LIMIT 100 OFFSET ?")?;
        let rows = entries.query_map(
            params![p.id, folder.to_string_lossy(), offset.max(0)],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u64>(1)?,
                    r.get::<_, u64>(2)?,
                ))
            },
        )?;
        let mut children = Vec::new();
        for r in rows {
            let (path, bytes, files) = r?;
            if Path::new(&path).parent() == Some(folder.as_path()) {
                children.push(json!({"path":path,"bytes":bytes,"files":files}));
            }
        }
        children.truncate(100);
        Ok(
            json!({"path":folder,"root":root,"parent":if folder==root{None}else{folder.parent()},"folders":children,"total":total,"offset":offset.max(0)}),
        )
    }
    pub fn map(&self, path: &str, offset: i64) -> Result<Value> {
        let p = self.progress.lock().clone();
        if p.id == 0 {
            return Ok(
                json!({"path":"","root":"","parent":null,"crumbs":[],"entries":[],"tiles":[],"total":0,"bytes":0,"files":0,"offset":0,"remainder":{"bytes":0,"count":0}}),
            );
        }
        let root = PathBuf::from(&p.root);
        let folder = if path.is_empty() {
            root.clone()
        } else {
            PathBuf::from(path)
        };
        if !folder.starts_with(&root)
            || folder
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            bail!("Папка вне текущего анализа")
        }
        let db = self.db.lock();
        let items = "SELECT path,bytes,files,'folder' AS kind,0 AS id,'folder' AS category,0 AS protected FROM folders WHERE scan=?1 AND parent=?2 UNION ALL SELECT path,size AS bytes,1 AS files,'file' AS kind,id,category,protected FROM files WHERE scan=?1 AND parent=?2";
        let (total, bytes, files): (u64, u64, u64) = db.query_row(
            &format!(
                "SELECT COUNT(*),COALESCE(SUM(bytes),0),COALESCE(SUM(files),0) FROM ({items})"
            ),
            params![p.id, folder.to_string_lossy()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let offset = if total == 0 {
            0
        } else {
            offset.max(0).min(((total - 1) / 60 * 60) as i64)
        };
        let query = |limit: i64, offset: i64| -> Result<Vec<Value>> {
            Ok(db.prepare(&format!("SELECT * FROM ({items}) ORDER BY bytes DESC,path LIMIT ?3 OFFSET ?4"))?
                .query_map(params![p.id,folder.to_string_lossy(),limit,offset],|r|Ok(json!({"path":r.get::<_,String>(0)?,"bytes":r.get::<_,u64>(1)?,"files":r.get::<_,u64>(2)?,"kind":r.get::<_,String>(3)?,"id":r.get::<_,i64>(4)?,"category":r.get::<_,String>(5)?,"protected":r.get::<_,bool>(6)?})))?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        };
        let tiles = query(120, 0)?;
        let shown: u64 = tiles.iter().map(|v| v["bytes"].as_u64().unwrap_or(0)).sum();
        let mut crumbs: Vec<_> = folder
            .ancestors()
            .take_while(|v| v.starts_with(&root))
            .map(|v| v.to_string_lossy().to_string())
            .collect();
        crumbs.reverse();
        Ok(
            json!({"path":folder,"root":root,"parent":if folder==root{None}else{folder.parent()},"crumbs":crumbs,"entries":query(60,offset)?,"tiles":tiles,"total":total,"bytes":bytes,"files":files,"offset":offset,"remainder":{"bytes":bytes.saturating_sub(shown),"count":total.saturating_sub(120)}}),
        )
    }
    fn file(&self, id: i64) -> Result<SpaceFile> {
        let scan = self.progress.lock().id;
        Ok(self
            .db
            .lock()
            .query_row(
                &format!("SELECT {COLS} FROM files WHERE id=? AND scan=?"),
                params![id, scan],
                row,
            )
            .context("Файл отсутствует в текущем анализе")?)
    }
    fn forget(&self, path: &Path) -> Result<()> {
        let scan = self.progress.lock().id;
        let removed = {
            let mut db = self.db.lock();
            let tx = db.transaction()?;
            let record:Option<(u64,String)>=tx.query_row("SELECT size,(SELECT root FROM scans WHERE id=?) FROM files WHERE scan=? AND path=?",params![scan,scan,path.to_string_lossy()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            if let Some((size, root)) = &record {
                tx.execute(
                    "DELETE FROM files WHERE scan=? AND path=?",
                    params![scan, path.to_string_lossy()],
                )?;
                for parent in path
                    .ancestors()
                    .skip(1)
                    .take_while(|p| p.starts_with(Path::new(root)))
                {
                    tx.execute("UPDATE folders SET bytes=MAX(0,bytes-?),files=MAX(0,files-1) WHERE scan=? AND path=?",params![size,scan,parent.to_string_lossy()])?;
                }
                tx.execute(
                    "UPDATE scans SET files=MAX(0,files-1),bytes=MAX(0,bytes-?) WHERE id=?",
                    params![size, scan],
                )?;
            }
            tx.commit()?;
            record.map(|r| r.0)
        };
        if let Some(size) = removed {
            let mut p = self.progress.lock();
            p.files = p.files.saturating_sub(1);
            p.bytes = p.bytes.saturating_sub(size);
        }
        Ok(())
    }
    pub fn recover(&self) -> Result<()> {
        let entries: Vec<(i64, String, String, String, String, String)> = {
            let db = self.db.lock();
            let mut q=db.prepare("SELECT id,original,vault,root,hash,state FROM trash WHERE state IN('pending','stored','restoring')")?;
            q.query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?
        };
        for (id, original, vault, root, hash, state) in entries {
            let original = Path::new(&original);
            let vault = Path::new(&vault);
            let root = Path::new(&root);
            let Some(parent) = original.parent() else {
                continue;
            };
            let vault_dir = parent.join(VAULT);
            if canonical(root).ok().as_deref() != Some(root)
                || canonical(&vault_dir).ok().as_deref() != Some(vault_dir.as_path())
                || vault.parent() != Some(vault_dir.as_path())
                || !original.starts_with(root)
                || original.is_symlink()
                || vault.is_symlink()
                || !original
                    .parent()
                    .and_then(|p| canonical(p).ok())
                    .is_some_and(|p| p.starts_with(root))
            {
                continue;
            }
            if state == "stored" && vault.exists() && !original.exists() {
                self.forget(original)?;
                continue;
            }
            let original_valid = original.exists()
                && crate::analysis::fingerprint(original).ok().as_deref() == Some(hash.as_str());
            let vault_valid = vault.exists()
                && crate::analysis::fingerprint(vault).ok().as_deref() == Some(hash.as_str());
            let next = if state == "pending" {
                if original_valid && vault_valid {
                    if crate::analysis::fingerprint(original).ok().as_deref() != Some(hash.as_str())
                        || fs::remove_file(vault).is_err()
                    {
                        continue;
                    }
                    "cancelled"
                } else if original_valid && !vault.exists() {
                    "cancelled"
                } else if !original.exists() && vault_valid {
                    self.forget(original)?;
                    "stored"
                } else {
                    continue;
                }
            } else if original_valid && !vault.exists() {
                "restored"
            } else if state == "restoring" && original_valid && vault_valid {
                if crate::analysis::fingerprint(original).ok().as_deref() != Some(hash.as_str())
                    || fs::remove_file(vault).is_err()
                {
                    continue;
                }
                "restored"
            } else if !original.exists() && vault_valid {
                "stored"
            } else {
                continue;
            };
            self.db
                .lock()
                .execute("UPDATE trash SET state=? WHERE id=?", params![next, id])?;
        }
        Ok(())
    }
    pub fn quarantine(&self, ids: &[i64], scan: i64) -> Result<Value> {
        let _guard = self.mutation.lock();
        let p = self.progress.lock().clone();
        if p.state == "running" || p.id != scan {
            bail!("Дождитесь окончания анализа и обновите список")
        }
        if Self::exclusions_pending(&self.db.lock())? {
            bail!("Исключения изменились. Повторите анализ перед очисткой")
        }
        if ids.is_empty() || ids.len() > 500 {
            bail!("Выберите от 1 до 500 файлов")
        }
        self.recover()?;
        let root = canonical(Path::new(&p.root))?;
        let mut done = 0;
        let mut errors = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for id in ids {
            if !seen.insert(*id) {
                continue;
            }
            let result = (|| -> Result<()> {
                let file = self
                    .db
                    .lock()
                    .query_row(
                        &format!("SELECT {COLS} FROM files WHERE id=? AND scan=?"),
                        params![id, scan],
                        row,
                    )
                    .context("Файл отсутствует в текущем анализе")?;
                let path = Path::new(&file.path);
                if file.protected || self.protected(path) {
                    bail!("Системный или служебный файл защищён")
                }
                let pending:i64=self.db.lock().query_row("SELECT COUNT(*) FROM trash WHERE original=? AND state IN('pending','restoring')",[&file.path],|r|r.get(0))?;
                if pending > 0 {
                    bail!("Для файла есть незавершённый перенос. Сначала проверьте корзину")
                }
                if !path.starts_with(&root)
                    || path.is_symlink()
                    || canonical(path.parent().context("Нет родительской папки")?)?
                        != normal(path.parent().unwrap())
                {
                    bail!("Путь изменился или содержит внешнюю ссылку")
                }
                let meta = fs::metadata(path)?;
                if !meta.is_file()
                    || meta.len() != file.size
                    || file.stamp == 0
                    || timestamp(&meta) != file.stamp
                {
                    bail!("Файл изменился. Повторите анализ")
                }
                let hash = crate::analysis::fingerprint(path)?;
                let vault = path.parent().context("Нет родительской папки")?.join(VAULT);
                if vault.exists() && (vault.is_symlink() || canonical(&vault)? != vault) {
                    bail!("Папка корзины содержит ссылку")
                }
                fs::create_dir_all(&vault)?;
                let target = vault.join(format!(
                    "{}{}",
                    uuid::Uuid::new_v4(),
                    path.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| format!(".{e}"))
                        .unwrap_or_default()
                ));
                let op = {
                    let db = self.db.lock();
                    db.execute("INSERT INTO trash(original,vault,root,size,hash,state,created) VALUES(?,?,?,?,?,'pending',?)",params![file.path,target.to_string_lossy(),root.to_string_lossy(),file.size,hash,now()])?;
                    db.last_insert_rowid()
                };
                crate::library::safe_transfer(path, &target, &hash)?;
                self.db
                    .lock()
                    .execute("UPDATE trash SET state='stored' WHERE id=?", [op])?;
                self.forget(path)?;
                Ok(())
            })();
            match result {
                Ok(()) => done += 1,
                Err(e) => errors.push(json!({"id":id,"message":e.to_string()})),
            }
        }
        Ok(
            json!({"done":done,"errors":errors,"note":"Файлы сохранены в корзине на исходном диске. Свободное место пока не изменилось."}),
        )
    }
    pub fn trash(&self, offset: i64) -> Result<Value> {
        let db = self.db.lock();
        let total: u64 = db.query_row(
            "SELECT COUNT(*) FROM trash WHERE state IN('stored','pending','restoring')",
            [],
            |r| r.get(0),
        )?;
        let offset = if total == 0 {
            0
        } else {
            offset
                .max(0)
                .min(((total - 1) / PAGE as u64 * PAGE as u64) as i64)
        };
        let rows=db.prepare("SELECT id,original,size,state,created FROM trash WHERE state IN('stored','pending','restoring') ORDER BY id DESC LIMIT ? OFFSET ?")?.query_map(params![PAGE,offset],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"path":r.get::<_,String>(1)?,"size":r.get::<_,u64>(2)?,"state":r.get::<_,String>(3)?,"created":r.get::<_,u64>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!({"files":rows,"total":total,"offset":offset,"page_size":PAGE}))
    }
    pub fn restore(&self, ids: &[i64]) -> Result<Value> {
        let _guard = self.mutation.lock();
        if self.progress.lock().state == "running" {
            bail!("Сначала остановите анализ")
        }
        if ids.is_empty() || ids.len() > 500 {
            bail!("Выберите от 1 до 500 файлов")
        }
        self.recover()?;
        let mut done = 0;
        let mut errors = Vec::new();
        for id in ids {
            let result = (|| -> Result<()> {
                let (original,vault,root,hash):(String,String,String,String)=self.db.lock().query_row("SELECT original,vault,root,hash FROM trash WHERE id=? AND state IN('stored','pending')",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
                let source = Path::new(&vault);
                let target = Path::new(&original);
                let root = PathBuf::from(root);
                if canonical(&root)? != root
                    || source.parent() != target.parent().map(|p| p.join(VAULT)).as_deref()
                    || canonical(source.parent().context("Нет папки корзины")?)?
                        != target
                            .parent()
                            .context("Нет родительской папки")?
                            .join(VAULT)
                    || source.is_symlink()
                    || !target.starts_with(&root)
                {
                    bail!("Путь восстановления изменился")
                }
                let parent = target.parent().context("Нет родительской папки")?;
                if !canonical(parent)?.starts_with(&root) {
                    bail!("Родительская папка содержит внешнюю ссылку")
                }
                if target.exists() {
                    bail!("Имя уже занято. Существующий файл сохранён")
                }
                if crate::analysis::fingerprint(source)? != hash {
                    bail!("Копия не прошла проверку SHA-256")
                }
                self.db
                    .lock()
                    .execute("UPDATE trash SET state='restoring' WHERE id=?", [id])?;
                crate::library::safe_transfer(source, target, &hash)?;
                self.db
                    .lock()
                    .execute("UPDATE trash SET state='restored' WHERE id=?", [id])?;
                Ok(())
            })();
            match result {
                Ok(()) => done += 1,
                Err(e) => errors.push(json!({"id":id,"message":e.to_string()})),
            }
        }
        Ok(json!({"done":done,"errors":errors}))
    }
    pub fn reveal(&self, id: i64) -> Result<Value> {
        let file = self.file(id)?;
        reveal_file(Path::new(&file.path))?;
        Ok(json!({"opened":true}))
    }
    pub fn reveal_trash(&self, id: i64) -> Result<Value> {
        let(original,vault):(String,String)=self.db.lock().query_row("SELECT original,vault FROM trash WHERE id=? AND state IN('pending','stored','restoring')",[id],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let expected = Path::new(&original)
            .parent()
            .context("Некорректный исходный путь")?
            .join(VAULT);
        let vault = Path::new(&vault);
        if vault.parent() != Some(expected.as_path())
            || canonical(&expected)? != expected
            || !vault.is_file()
            || vault.is_symlink()
        {
            bail!("Сохранённая копия недоступна или путь изменился")
        }
        reveal_file(vault)?;
        Ok(json!({"opened":true}))
    }
}

fn folder_pattern(folder: &str) -> String {
    let escaped = folder
        .trim_end_matches(['\\', '/'])
        .replace('!', "!!")
        .replace('%', "!%")
        .replace('_', "!_");
    if folder.is_empty() {
        String::new()
    } else {
        format!("{escaped}{}%", std::path::MAIN_SEPARATOR)
    }
}
pub fn drives() -> Vec<Value> {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetDriveTypeW(root: *const u16) -> u32;
        }
        (b'A'..=b'Z').filter_map(|letter|{
            let path=format!("{}:\\",letter as char);let wide:Vec<u16>=path.encode_utf16().chain(Some(0)).collect();
            let kind=unsafe{GetDriveTypeW(wide.as_ptr())};if kind!=2&&kind!=3{return None}
            let total=fs2::total_space(&path).ok()?;let free=fs2::available_space(&path).ok()?;
            Some(json!({"path":path,"total":total,"free":free,"used":total.saturating_sub(free),"removable":kind==2,"optimizable":kind==3}))
        }).collect()
    }
    #[cfg(not(windows))]
    {
        fs2::total_space("/").ok().map(|total|{let free=fs2::available_space("/").unwrap_or(0);vec![json!({"path":"/","total":total,"free":free,"used":total.saturating_sub(free),"optimizable":false})]}).unwrap_or_default()
    }
}
pub fn is_administrator() -> bool {
    #[cfg(windows)]
    {
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn IsUserAnAdmin() -> i32;
        }
        unsafe { IsUserAnAdmin() != 0 }
    }
    #[cfg(not(windows))]
    {
        false
    }
}
#[cfg(windows)]
pub fn restart_elevated(data: &Path) -> Result<()> {
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: *mut std::ffi::c_void,
            operation: *const u16,
            file: *const u16,
            params: *const u16,
            directory: *const u16,
            show: i32,
        ) -> isize;
    }
    let executable = std::env::current_exe()?;
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let op = wide("runas");
    let file = wide(&executable.to_string_lossy());
    let args = wide(&format!(
        "--data {} --elevated-wait",
        quote_windows_arg(&data.to_string_lossy())
    ));
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            file.as_ptr(),
            args.as_ptr(),
            std::ptr::null(),
            1,
        )
    };
    if result <= 32 {
        bail!("Запуск с правами администратора отменён или недоступен (код {result})")
    }
    Ok(())
}
pub fn quote_windows_arg(value: &str) -> String {
    let mut out = String::from("\"");
    let mut slashes = 0;
    for c in value.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        if c == '\"' {
            out.push_str(&"\\".repeat(slashes * 2 + 1));
            out.push(c);
        } else {
            out.push_str(&"\\".repeat(slashes));
            out.push(c);
        }
        slashes = 0;
    }
    out.push_str(&"\\".repeat(slashes * 2));
    out.push('"');
    out
}
pub fn optimize_drive(drive: &str, analyze: bool) -> Result<Value> {
    if drive.len() != 2 || !drive.as_bytes()[0].is_ascii_alphabetic() || drive.as_bytes()[1] != b':'
    {
        bail!("Выберите локальный диск")
    }
    let drive = drive.to_uppercase();
    if !drives()
        .iter()
        .any(|d| d["path"] == format!("{drive}\\") && d["optimizable"] == true)
    {
        bail!("Диск недоступен для оптимизации")
    }
    launch_elevated_optimizer(&drive, analyze)?;
    Ok(
        json!({"launched":true,"mode":if analyze{"analyze"}else{"optimize"},"message":"Запрошен запуск системного окна. Подтвердите UAC; результат и ход работы отображаются Windows."}),
    )
}
#[cfg(windows)]
fn launch_elevated_optimizer(drive: &str, analyze: bool) -> Result<()> {
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: *mut std::ffi::c_void,
            operation: *const u16,
            file: *const u16,
            params: *const u16,
            directory: *const u16,
            show: i32,
        ) -> isize;
    }
    let system = std::env::var_os("SystemRoot").context("Не найдена папка Windows")?;
    let executable = PathBuf::from(system).join("System32/defrag.exe");
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let operation = wide("runas");
    let file = wide(&executable.to_string_lossy());
    let arguments = wide(&format!(
        "{drive} {} /U /V",
        if analyze { "/A" } else { "/O" }
    ));
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            arguments.as_ptr(),
            std::ptr::null(),
            1,
        )
    };
    if result <= 32 {
        bail!("Windows не запустила оптимизацию (код {result}); возможно, запрос отменён")
    }
    Ok(())
}
#[cfg(not(windows))]
fn launch_elevated_optimizer(_: &str, _: bool) -> Result<()> {
    bail!("Оптимизация доступна на Windows")
}

pub fn system_page(page: &str) -> Result<Value> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let windows = PathBuf::from(std::env::var_os("SystemRoot").context("Не найдена Windows")?);
        match page {
            "optimize" => {
                std::process::Command::new(windows.join("System32/dfrgui.exe"))
                    .creation_flags(0x08000000)
                    .spawn()?;
            }
            "storage" | "apps" | "recycle" => {
                let target = match page {
                    "storage" => "ms-settings:storagesense",
                    "apps" => "ms-settings:appsfeatures",
                    _ => "shell:RecycleBinFolder",
                };
                std::process::Command::new(windows.join("explorer.exe"))
                    .arg(target)
                    .creation_flags(0x08000000)
                    .spawn()?;
            }
            _ => bail!("Неизвестный раздел"),
        }
        Ok(json!({"opened":true}))
    }
    #[cfg(not(windows))]
    {
        let _ = page;
        bail!("Системные инструменты доступны на Windows")
    }
}
fn reveal_file(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new(
            PathBuf::from(std::env::var_os("SystemRoot").context("Не найдена Windows")?)
                .join("explorer.exe"),
        )
        .arg(format!("/select,{}", path.to_string_lossy()))
        .creation_flags(0x08000000)
        .spawn()?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        bail!("Проводник доступен на Windows")
    }
}
