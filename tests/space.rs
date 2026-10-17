use photosort::space::Space;
use rusqlite::{Connection, params};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

struct Fixture {
    temp: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
    space: Arc<Space>,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("files");
        let data = temp.path().join("index");
        fs::create_dir(&root).unwrap();
        let space = Space::open(&data).unwrap();
        let root = fs::canonicalize(root).unwrap();
        #[cfg(windows)]
        let root = PathBuf::from(
            root.to_string_lossy()
                .trim_start_matches(r"\\?\")
                .to_string(),
        );
        Self {
            temp,
            root,
            data,
            space,
        }
    }
    fn write(&self, path: &str, content: &[u8]) -> PathBuf {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        path
    }
    fn scan(&self) {
        self.space.start(self.root.to_str().unwrap()).unwrap();
        self.wait();
        assert_eq!(self.space.progress.lock().state, "complete");
    }
    fn wait(&self) {
        let start = Instant::now();
        while self.space.progress.lock().state == "running" {
            assert!(start.elapsed() < Duration::from_secs(30));
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn files(&self) -> serde_json::Value {
        self.space.files("all", "", 0, "", "").unwrap()
    }
    fn id(&self, name: &str) -> i64 {
        self.files()["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| Path::new(f["path"].as_str().unwrap()).file_name().unwrap() == name)
            .unwrap()["id"]
            .as_i64()
            .unwrap()
    }
    fn db(&self) -> Connection {
        Connection::open(self.data.join("space.sqlite3")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.space.close();
    }
}

#[test]
fn scan_all_file_types_counts_nested_folders_and_preserves_sources() {
    let f = Fixture::new();
    f.write("docs/Отчёт.PDF", b"not a decoded PDF");
    f.write("docs/nested/a.bin", b"1234");
    f.write("clips/clip.mp4", b"invalid video still counted");
    f.write(".photosort-space-trash/hidden.bin", b"ignored");
    f.write(".photosort-trash/hidden.jpg", b"ignored");
    f.scan();
    assert_eq!(f.files()["total"], 3);
    let p = f.space.progress.lock().clone();
    assert_eq!(p.files, 3);
    assert_eq!(p.errors, 0);
    let folders = f.space.folders("", 0).unwrap();
    assert_eq!(folders["folders"].as_array().unwrap().len(), 2);
    let docs = folders["folders"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["path"].as_str().unwrap().ends_with("docs"))
        .unwrap();
    assert_eq!(docs["files"], 2);
    assert_eq!(docs["bytes"], 21);
    assert_eq!(
        f.space.files("all", "ОТЧЁТ", 0, "", "").unwrap()["total"],
        1
    );
    assert_eq!(
        f.space.files("all", "", 0, "", "video").unwrap()["total"],
        1
    );
    assert!(f.root.join("docs/Отчёт.PDF").exists());
}
#[test]
fn large_files_and_pagination_do_not_decode_or_truncate_results() {
    let f = Fixture::new();
    for i in 0..75 {
        f.write(&format!("f{i:03}.bin"), b"file");
    }
    let huge = f.write("large.iso", b"");
    fs::File::options()
        .write(true)
        .open(huge)
        .unwrap()
        .set_len(120 * 1024 * 1024)
        .unwrap();
    f.scan();
    assert_eq!(f.space.files("large", "", 0, "", "").unwrap()["total"], 1);
    let first = f.files();
    let second = f.space.files("all", "", 60, "", "").unwrap();
    assert_eq!(first["total"], 76);
    assert_eq!(first["files"].as_array().unwrap().len(), 60);
    assert_eq!(second["files"].as_array().unwrap().len(), 16);
}
#[test]
fn folder_filter_escapes_sql_wildcards() {
    let f = Fixture::new();
    f.write("one_%/a.bin", b"a");
    f.write("one_XX/b.bin", b"b");
    f.scan();
    let folder = f.root.join("one_%");
    assert_eq!(
        f.space
            .files("all", "", 0, folder.to_str().unwrap(), "")
            .unwrap()["total"],
        1
    );
    assert!(f.space.folders(f.temp.path().to_str().unwrap(), 0).is_err());
}
#[test]
fn quarantine_and_restore_never_overwrite_and_update_map() {
    let f = Fixture::new();
    let path = f.write("docs/a.txt", b"original");
    f.scan();
    let id = f.id("a.txt");
    let scan = f.space.progress.lock().id;
    let result = f.space.quarantine(&[id], scan).unwrap();
    assert_eq!(result["done"], 1);
    assert!(!path.exists());
    assert_eq!(f.files()["total"], 0);
    assert_eq!(f.space.progress.lock().bytes, 0);
    let trash = f.space.trash(0).unwrap();
    let tid = trash["files"][0]["id"].as_i64().unwrap();
    fs::write(&path, b"replacement").unwrap();
    assert_eq!(f.space.restore(&[tid]).unwrap()["done"], 0);
    assert_eq!(fs::read(&path).unwrap(), b"replacement");
    fs::remove_file(&path).unwrap();
    assert_eq!(f.space.restore(&[tid]).unwrap()["done"], 1);
    assert_eq!(fs::read(&path).unwrap(), b"original");
    assert!(
        f.space.trash(0).unwrap()["files"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn changed_files_and_obsolete_scan_are_rejected() {
    let f = Fixture::new();
    let path = f.write("a.bin", b"before");
    f.scan();
    let id = f.id("a.bin");
    let scan = f.space.progress.lock().id;
    fs::write(&path, b"changed content").unwrap();
    assert_eq!(f.space.quarantine(&[id], scan).unwrap()["done"], 0);
    assert!(path.exists());
    f.scan();
    assert!(f.space.quarantine(&[id], scan).is_err());
}
#[test]
fn crash_recovery_preserves_replacement_and_original_copy() {
    let f = Fixture::new();
    let path = f.write("a.bin", b"original");
    f.scan();
    let hash = photosort::analysis::fingerprint(&path).unwrap();
    let root = f.space.progress.lock().root.clone();
    let vault = Path::new(&root).join(".photosort-space-trash/copy.bin");
    fs::create_dir_all(vault.parent().unwrap()).unwrap();
    fs::copy(&path, &vault).unwrap();
    f.db().execute("INSERT INTO trash(original,vault,root,size,hash,state,created) VALUES(?,?,?,?,?,'pending',1)",params![path.to_string_lossy(),vault.to_string_lossy(),root,8,hash]).unwrap();
    fs::write(&path, b"replacement").unwrap();
    f.space.recover().unwrap();
    f.space.recover().unwrap();
    assert_eq!(fs::read(&vault).unwrap(), b"original");
    assert_eq!(fs::read(&path).unwrap(), b"replacement");
    assert_eq!(f.space.trash(0).unwrap()["files"][0]["state"], "pending");
    fs::remove_file(&path).unwrap();
    f.space.recover().unwrap();
    assert_eq!(f.space.trash(0).unwrap()["files"][0]["state"], "stored");
    assert_eq!(f.files()["total"], 0);
}
#[test]
fn reopened_index_restores_interrupted_restore_journal() {
    let f = Fixture::new();
    let path = f.write("a.bin", b"original");
    f.scan();
    let id = f.id("a.bin");
    let scan = f.space.progress.lock().id;
    f.space.quarantine(&[id], scan).unwrap();
    let (tid, vault): (i64, String) = f
        .db()
        .query_row("SELECT id,vault FROM trash", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    fs::copy(&vault, &path).unwrap();
    f.db()
        .execute("UPDATE trash SET state='restoring' WHERE id=?", [tid])
        .unwrap();
    f.space.close();
    let reopened = Space::open(&f.data).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"original");
    assert!(!Path::new(&vault).exists());
    assert!(
        reopened.trash(0).unwrap()["files"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    reopened.close();
}
#[test]
fn optimizer_rejects_arbitrary_command_arguments() {
    for input in ["C: /D", "C:\\Windows", "C:;whoami", "\\\\server\\share", ""] {
        assert!(photosort::space::optimize_drive(input, false).is_err());
    }
}

#[test]
fn elevation_quotes_spaces_and_trailing_backslashes() {
    assert_eq!(
        photosort::space::quote_windows_arg(r"C:\My Data"),
        r#""C:\My Data""#
    );
    assert_eq!(photosort::space::quote_windows_arg(r"C:\"), r#""C:\\""#);
    assert_eq!(photosort::space::quote_windows_arg("x\"y"), "\"x\\\"y\"");
}

#[test]
fn map_paginates_all_immediate_folders() {
    let f = Fixture::new();
    for i in 0..105 {
        f.write(&format!("folder-{i:03}/one.bin"), b"x");
    }
    f.scan();
    let first = f.space.folders("", 0).unwrap();
    let second = f.space.folders("", 100).unwrap();
    assert_eq!(first["total"], 105);
    assert_eq!(first["folders"].as_array().unwrap().len(), 100);
    assert_eq!(second["folders"].as_array().unwrap().len(), 5);
    assert_ne!(first["folders"][0]["path"], second["folders"][0]["path"]);
}

#[test]
fn operating_system_and_recovery_files_are_never_cleanup_candidates() {
    let f = Fixture::new();
    let path = f.write("Recovery/boot-data.bin", b"critical");
    f.write("NTUSER.DAT", b"profile");
    f.scan();
    let scan = f.space.progress.lock().id;
    for name in ["boot-data.bin", "NTUSER.DAT"] {
        let id = f.id(name);
        assert_eq!(f.space.quarantine(&[id], scan).unwrap()["done"], 0);
    }
    assert!(path.exists());
    assert_eq!(f.files()["total"], 2);
}

#[test]
fn same_size_edit_within_one_second_is_rejected() {
    let f = Fixture::new();
    let path = f.write("precise.bin", b"before");
    let time =
        std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000) + Duration::from_millis(100);
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(time))
        .unwrap();
    f.scan();
    let scan = f.space.progress.lock().id;
    let id = f.id("precise.bin");
    fs::write(&path, b"after!").unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(time + Duration::from_micros(100)))
        .unwrap();
    assert_eq!(f.space.quarantine(&[id], scan).unwrap()["done"], 0);
    assert_eq!(fs::read(path).unwrap(), b"after!");
}

#[test]
fn space_routes_preserve_media_and_share_persistent_theme() {
    use std::io::{Read, Write};
    let temp = tempfile::tempdir().unwrap();
    let library = photosort::library::Library::open(temp.path()).unwrap();
    let url = photosort::server::serve(library.clone(), 0).unwrap();
    let host = url.trim_start_matches("http://");
    let request = |path: &str, body: Option<&str>, token: &str| -> String {
        let mut stream = std::net::TcpStream::connect(host).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let text = if let Some(body) = body {
            format!(
                "POST {path} HTTP/1.1\r\nHost: {host}\r\nX-PhotoSort-Token: {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        } else {
            format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
        };
        stream.write_all(text.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    };
    let session = request("/api/session", None, "");
    let json: serde_json::Value =
        serde_json::from_str(session.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    let token = json["token"].as_str().unwrap();
    assert!(request("/", None, "").contains("space.js"));
    for asset in ["/icons.js", "/map-layout.js", "/space-refined.css"] {
        assert!(
            request(asset, None, "").starts_with("HTTP/1.1 200"),
            "Missing UI asset: {asset}"
        );
    }
    assert!(request("/photos", None, "").contains("duplicates-dialog"));
    assert!(request("/api/space/theme", Some(r#"{"theme":"dark"}"#), "").contains("403"));
    assert!(
        request("/api/space/theme", Some(r#"{"theme":"dark"}"#), token).starts_with("HTTP/1.1 200")
    );
    assert_eq!(library.settings().theme, "dark");
    assert!(
        request("/api/space/theme", Some(r#"{"theme":"invalid"}"#), token)
            .starts_with("HTTP/1.1 400")
    );
    assert_eq!(library.settings().theme, "dark");
    assert!(request("/api/space/exclusions", None, "").contains("\"paths\":[]"));
    assert!(request("/api/space/exclusions", Some(r#"{"paths":[]}"#), "").contains("403"));
    assert!(
        request("/api/space/exclusions", Some(r#"{"paths":[]}"#), token)
            .starts_with("HTTP/1.1 200")
    );
    assert!(
        request(
            "/api/space/exclusions",
            Some(r#"{"paths":["relative"]}"#),
            token
        )
        .starts_with("HTTP/1.1 400")
    );
    assert!(
        request("/api/space/exclusions", Some(r#"{"paths":[42]}"#), token)
            .starts_with("HTTP/1.1 400")
    );
    library.close();
    thread::sleep(Duration::from_millis(250));
}

#[test]
fn trash_pagination_keeps_older_entries_accessible() {
    let f = Fixture::new();
    let mut db = f.db();
    let tx = db.transaction().unwrap();
    for i in 0..601 {
        tx.execute("INSERT INTO trash(original,vault,root,size,hash,state,created) VALUES(?,'','','1','','stored',1)", [format!("file-{i}")]).unwrap();
    }
    tx.commit().unwrap();
    let first = f.space.trash(0).unwrap();
    let last = f.space.trash(600).unwrap();
    assert_eq!(first["total"], 601);
    assert_eq!(first["files"].as_array().unwrap().len(), 60);
    assert_eq!(last["files"].as_array().unwrap().len(), 1);
    assert_eq!(last["files"][0]["path"], "file-0");
    assert_eq!(f.space.trash(i64::MAX).unwrap()["offset"], 600);
    assert_eq!(f.space.trash(-1).unwrap()["offset"], 0);
    db.execute("UPDATE trash SET state='restored' WHERE id=1", [])
        .unwrap();
    let adjusted = f.space.trash(600).unwrap();
    assert_eq!(adjusted["offset"], 540);
    assert_eq!(adjusted["files"].as_array().unwrap().len(), 60);
}

#[test]
fn map_includes_direct_files_and_keeps_exact_remainder_proportions() {
    let f = Fixture::new();
    f.write("loose.bin", &[1; 50]);
    for i in 0..131 {
        f.write(&format!("folder-{i:03}/one.bin"), b"x");
    }
    f.scan();
    let map = f.space.map("", 0).unwrap();
    assert_eq!(map["total"], 132);
    assert_eq!(map["bytes"], 181);
    assert_eq!(map["files"], 132);
    assert_eq!(map["entries"][0]["kind"], "file");
    assert_eq!(map["entries"][0]["bytes"], 50);
    let shown: u64 = map["tiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["bytes"].as_u64().unwrap())
        .sum();
    assert_eq!(shown + map["remainder"]["bytes"].as_u64().unwrap(), 181);
    assert_eq!(map["remainder"]["count"], 12);
    assert_eq!(
        f.space.map("", 120).unwrap()["entries"]
            .as_array()
            .unwrap()
            .len(),
        12
    );
    let folder = f.root.join("folder-000");
    let child = f.space.map(folder.to_str().unwrap(), 0).unwrap();
    assert_eq!(child["entries"][0]["kind"], "file");
    assert_eq!(child["crumbs"].as_array().unwrap().len(), 2);
    assert_eq!(child["bytes"], 1);
    assert!(
        f.space
            .map(f.root.join("../index").to_str().unwrap(), 0)
            .is_err()
    );
}

#[test]
fn map_migrates_v06_parent_index_without_rescanning_or_losing_trash() {
    let f = Fixture::new();
    f.write("docs/a.bin", b"content");
    f.write("docs/b.bin", b"recoverable");
    f.scan();
    let scan = f.space.progress.lock().id;
    f.space.quarantine(&[f.id("b.bin")], scan).unwrap();
    let stored_id = f.space.trash(0).unwrap()["files"][0]["id"]
        .as_i64()
        .unwrap();
    f.space.close();
    let db = f.db();
    db.execute_batch("DROP INDEX space_file_parent; ALTER TABLE files DROP COLUMN parent;")
        .unwrap();
    drop(db);
    let reopened = Space::open(&f.data).unwrap();
    let map = reopened
        .map(f.root.join("docs").to_str().unwrap(), 0)
        .unwrap();
    assert_eq!(map["entries"][0]["bytes"], 7);
    assert_eq!(map["files"], 1);
    assert_eq!(reopened.trash(0).unwrap()["files"][0]["id"], stored_id);
    assert_eq!(reopened.restore(&[stored_id]).unwrap()["done"], 1);
    assert_eq!(fs::read(f.root.join("docs/b.bin")).unwrap(), b"recoverable");
    reopened.close();
}

#[test]
fn exclusions_prune_nested_folders_and_exact_files_without_prefix_collisions() {
    let f = Fixture::new();
    let secret = f.write("Личное/nested/secret.bin", b"secret");
    f.write("Личное-other/keep.bin", b"keep");
    let single = f.write("single.bin", b"single");
    f.write("single.bin-copy", b"keep too");
    let folder = secret.parent().unwrap().parent().unwrap();
    f.space
        .save_exclusions(&[
            folder.to_string_lossy().into(),
            single.to_string_lossy().into(),
        ])
        .unwrap();
    f.scan();
    assert_eq!(f.files()["total"], 2);
    let map = f.space.map("", 0).unwrap();
    assert_eq!(map["files"], 2);
    assert_eq!(map["bytes"], 12);
    assert!(secret.exists() && single.exists());
    assert!(f.space.start(folder.to_str().unwrap()).is_err());
    assert_eq!(f.space.status().unwrap()["exclusions_pending"], false);
}

#[test]
fn exclusions_persist_deduplicate_and_cover_temporarily_missing_paths() {
    let f = Fixture::new();
    let missing = f.root.join("not-yet-created");
    let path = missing.to_string_lossy().into_owned();
    #[cfg(windows)]
    let duplicate = path.to_uppercase().replace('\\', "/") + "/";
    #[cfg(not(windows))]
    let duplicate = path.clone() + "/";
    f.space.save_exclusions(&[path.clone(), duplicate]).unwrap();
    assert_eq!(f.space.exclusions().unwrap().len(), 1);
    f.space.close();
    let reopened = Space::open(&f.data).unwrap();
    assert_eq!(reopened.exclusions().unwrap(), vec![path]);
    f.write("not-yet-created/private.bin", b"private");
    f.write("visible.bin", b"yes");
    reopened.start(f.root.to_str().unwrap()).unwrap();
    while reopened.progress.lock().state == "running" {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(reopened.progress.lock().files, 1);
    reopened.close();
}

#[test]
fn changing_exclusions_blocks_stale_cleanup_but_not_restore() {
    let f = Fixture::new();
    let kept = f.write("keep.bin", b"keep");
    let restored = f.write("restore.bin", b"restore");
    f.scan();
    let scan = f.space.progress.lock().id;
    let id = f.id("keep.bin");
    assert_eq!(
        f.space.quarantine(&[f.id("restore.bin")], scan).unwrap()["done"],
        1
    );
    let trash_id = f.space.trash(0).unwrap()["files"][0]["id"]
        .as_i64()
        .unwrap();
    f.space
        .save_exclusions(&[f.root.to_string_lossy().into()])
        .unwrap();
    assert_eq!(f.space.status().unwrap()["exclusions_pending"], true);
    assert!(
        f.space
            .quarantine(&[id], scan)
            .unwrap_err()
            .to_string()
            .contains("Исключения")
    );
    assert!(kept.exists());
    assert_eq!(f.space.restore(&[trash_id]).unwrap()["done"], 1);
    assert!(restored.exists());
    f.space.save_exclusions(&[]).unwrap();
    f.scan();
    assert_eq!(f.space.status().unwrap()["exclusions_pending"], false);
    assert_eq!(f.files()["total"], 2);
}

#[test]
fn exclusions_validate_atomically_and_cannot_change_during_analysis() {
    let f = Fixture::new();
    let path = f.root.join("private").to_string_lossy().into_owned();
    f.space.save_exclusions(&[path.clone()]).unwrap();
    assert!(
        f.space
            .save_exclusions(&[path.clone(), "relative/path".into()])
            .is_err()
    );
    assert!(f.space.save_exclusions(&vec![path.clone(); 257]).is_err());
    assert!(f.space.save_exclusions(&[String::new()]).is_err());
    assert_eq!(f.space.exclusions().unwrap(), vec![path]);
    f.space.progress.lock().state = "running".into();
    assert!(f.space.save_exclusions(&[]).is_err());
    f.space.progress.lock().state.clear();
    f.scan();
    f.space
        .save_exclusions(&f.space.exclusions().unwrap())
        .unwrap();
    assert_eq!(f.space.status().unwrap()["exclusions_pending"], false);
}

#[test]
fn cleanup_rejects_old_rows_after_an_interrupted_new_scan() {
    let f = Fixture::new();
    let path = f.write("private.bin", b"must stay");
    f.scan();
    let old = f.space.progress.lock().clone();
    let id = f.id("private.bin");
    f.space
        .save_exclusions(&[path.to_string_lossy().into_owned()])
        .unwrap();
    let db = f.db();
    db.execute(
        "INSERT INTO scans(root,state,started) VALUES(?,'error',0)",
        [f.root.to_string_lossy()],
    )
    .unwrap();
    let scan = db.last_insert_rowid();
    db.execute("UPDATE space_settings SET value=(SELECT value FROM space_settings WHERE key='exclusions_revision') WHERE key='scan_exclusions_revision'", []).unwrap();
    *f.space.progress.lock() = photosort::space::ScanProgress {
        id: scan,
        state: "error".into(),
        ..old
    };
    let result = f.space.quarantine(&[id], scan).unwrap();
    assert_eq!(result["done"], 0);
    assert!(
        result["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("текущем анализе")
    );
    assert_eq!(fs::read(path).unwrap(), b"must stay");
}
