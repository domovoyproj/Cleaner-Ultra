mod common;
use cleaner_ultra::{
    analysis::{self, BKTree, HammingIndex},
    library::Library,
};
use rusqlite::{Connection, params};
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

struct Fixture {
    library: Arc<Library>,
    root: PathBuf,
    _temp: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("build/rust-tests");
        fs::create_dir_all(&base).unwrap();
        let temp = tempfile::Builder::new()
            .prefix("test-")
            .tempdir_in(base)
            .unwrap();
        let root = temp.path().join("photos");
        fs::create_dir(&root).unwrap();
        let library = Library::open(&temp.path().join("index")).unwrap();
        Self {
            _temp: temp,
            root,
            library,
        }
    }
    fn image(&self, name: &str, seed: u32) -> PathBuf {
        let path = self.root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let image = image::RgbImage::from_fn(160, 120, |x, y| {
            image::Rgb([
                ((x * 3 + seed) % 256) as u8,
                ((y * 4 + seed) % 256) as u8,
                ((x + y + seed) % 256) as u8,
            ])
        });
        image.save(&path).unwrap();
        path
    }
    fn scan(&self) {
        self.library
            .start(self.root.to_str().unwrap(), false)
            .unwrap();
        self.wait();
        let progress = self.library.progress.lock().clone();
        assert_eq!(progress.errors, 0, "{}", progress.last_error);
    }
    fn wait(&self) {
        let start = Instant::now();
        while self.library.progress.lock().running {
            assert!(start.elapsed() < Duration::from_secs(90));
            thread::sleep(Duration::from_millis(20));
        }
    }
    fn photos(&self, view: &str) -> Vec<Value> {
        self.library.listing(view, "", 0, "name").unwrap()["photos"]
            .as_array()
            .unwrap()
            .clone()
    }
    fn db(&self) -> Connection {
        Connection::open(self.library.data.join("library.sqlite3")).unwrap()
    }
    fn movie(&self, name: &str, seconds: u32) -> PathBuf {
        let path = self.root.join(name);
        let result = std::process::Command::new(
            cleaner_ultra::video::tool("ffmpeg")
                .expect("Run packaging/fetch-ffmpeg.ps1 before video tests"),
        )
        .args([
            "-v",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=15",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
        ])
        .arg(seconds.to_string())
        .args([
            "-threads",
            "1",
            "-c:v",
            "mpeg4",
            "-q:v",
            "2",
            "-c:a",
            "aac",
            "-b:a",
            "96k",
            "-shortest",
            "-y",
        ])
        .arg(&path)
        .output()
        .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.library.close();
    }
}

#[test]
fn media_ranges_cover_seek_suffix_and_invalid_requests() {
    use cleaner_ultra::server::byte_range;
    assert_eq!(byte_range(None, 1000), Some((0, 1000, false)));
    assert_eq!(
        byte_range(Some("bytes=100-199"), 1000),
        Some((100, 100, true))
    );
    assert_eq!(byte_range(Some("bytes=900-"), 1000), Some((900, 100, true)));
    assert_eq!(byte_range(Some("bytes=-100"), 1000), Some((900, 100, true)));
    assert_eq!(
        byte_range(Some("bytes=900-9999"), 1000),
        Some((900, 100, true))
    );
    for range in [
        "bytes=1000-",
        "bytes=200-100",
        "bytes=-0",
        "bytes=1-2,5-6",
        "items=1-2",
        "bytes=a-b",
    ] {
        assert_eq!(byte_range(Some(range), 1000), None);
    }
}
#[test]
fn exclusive_copy_preserves_modified_time_and_conflicts() {
    let f = Fixture::new();
    let source = f.image("source.jpg", 8);
    let timestamp = std::time::UNIX_EPOCH + Duration::from_secs(1_500_000_000);
    fs::File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(timestamp))
        .unwrap();
    let hash = analysis::fingerprint(&source).unwrap();
    let target = f.root.join("copy.jpg");
    cleaner_ultra::library::copy_exclusive(&source, &target, &hash).unwrap();
    assert_eq!(
        fs::metadata(&source).unwrap().modified().unwrap(),
        fs::metadata(&target).unwrap().modified().unwrap()
    );
    assert!(cleaner_ultra::library::copy_exclusive(&source, &target, &hash).is_err());
    assert_eq!(analysis::fingerprint(&source).unwrap(), hash);
    assert_eq!(analysis::fingerprint(&target).unwrap(), hash);
}
#[test]
fn video_cache_cleanup_preserves_unowned_files_and_journal() {
    let f = Fixture::new();
    let mut settings = f.library.settings();
    settings.cache_mb = 64;
    f.library.save_settings(settings).unwrap();
    let owned = f
        .library
        .thumbs
        .join(format!("video-{}.webm", "a".repeat(64)));
    fs::File::create(&owned)
        .unwrap()
        .set_len(80 * 1024 * 1024)
        .unwrap();
    let unowned = f.library.thumbs.join("personal.webm");
    fs::write(&unowned, b"keep").unwrap();
    let unowned_jpeg = f.library.thumbs.join("personal.jpg");
    fs::write(&unowned_jpeg, b"keep this photo").unwrap();
    let note = f.library.data.join("journal-note");
    fs::write(&note, b"keep").unwrap();
    f.library.trim_cache().unwrap();
    assert!(!owned.exists());
    assert_eq!(fs::read(unowned).unwrap(), b"keep");
    assert_eq!(fs::read(unowned_jpeg).unwrap(), b"keep this photo");
    assert!(note.exists());
    assert!(f.library.data.join("library.sqlite3").exists());
}
#[test]
#[ignore = "Requires bundled FFmpeg; release pipeline runs this test"]
fn video_import_groups_export_trash_and_conflict_restore() {
    let f = Fixture::new();
    let source = f.movie("clip.MP4", 3);
    fs::copy(&source, f.root.join("duplicate.mp4")).unwrap();
    let recode = f.root.join("reencoded.avi");
    let result = std::process::Command::new(cleaner_ultra::video::tool("ffmpeg").unwrap())
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(&source)
        .args(["-c:v", "mpeg4", "-q:v", "5", "-threads", "1", "-y"])
        .arg(&recode)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    f.movie("longer.mp4", 6);
    f.image("clip.jpg", 7);
    f.scan();
    assert_eq!(f.photos("videos").len(), 4);
    assert_eq!(f.photos("photos").len(), 1);
    assert_eq!(f.photos("duplicates").len(), 2);
    let videos = f.photos("videos");
    let clip = videos.iter().find(|p| p["name"] == "clip.MP4").unwrap();
    let id = clip["id"].as_i64().unwrap();
    assert_eq!(clip["media_kind"], "video");
    assert_eq!(clip["width"], 320);
    assert_eq!(clip["height"], 180);
    assert!((clip["duration"].as_f64().unwrap() - 3.0).abs() < 0.1);
    assert_eq!(clip["video_codec"], "mpeg4");
    assert_eq!(f.photos("similar").len(), 3);
    assert!(f.photos("bursts").is_empty());
    let expected = analysis::fingerprint(&source).unwrap();
    let exported = f.root.parent().unwrap().join("export");
    f.library
        .export(&[id], exported.to_str().unwrap(), false)
        .unwrap();
    assert_eq!(
        analysis::fingerprint(&exported.join("clip.MP4")).unwrap(),
        expected
    );
    assert_ne!(
        cleaner_ultra::library::asset_key(&source),
        cleaner_ultra::library::asset_key(&f.root.join("clip.jpg"))
    );
    assert_eq!(f.library.move_photos(&[id], false).unwrap().done, vec![id]);
    assert!(!source.exists());
    let trash = f.library.video_source(id, false).unwrap();
    assert_eq!(analysis::fingerprint(&trash).unwrap(), expected);
    fs::write(&source, b"existing file").unwrap();
    assert_eq!(f.library.move_photos(&[id], true).unwrap().errors.len(), 1);
    assert_eq!(fs::read(&source).unwrap(), b"existing file");
    assert!(trash.exists());
    fs::remove_file(&source).unwrap();
    assert_eq!(f.library.undo().unwrap().done, vec![id]);
    assert_eq!(analysis::fingerprint(&source).unwrap(), expected);
    f.scan();
    assert_eq!(f.photos("videos").len(), 4);
    assert_eq!(f.library.photo(id).unwrap().id, id);
}
fn video_request(host: &str, method: &str, path: &str, range: Option<&str>) -> (String, Vec<u8>) {
    let mut connection = std::net::TcpStream::connect(host).unwrap();
    connection
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let range = range.map(|r| format!("Range: {r}\r\n")).unwrap_or_default();
    write!(
        connection,
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\n{range}Connection: close\r\n\r\n"
    )
    .unwrap();
    let mut bytes = Vec::new();
    connection.read_to_end(&mut bytes).unwrap();
    let boundary = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    (
        String::from_utf8(bytes[..boundary].to_vec()).unwrap(),
        bytes[boundary + 4..].to_vec(),
    )
}
#[test]
#[ignore = "Requires bundled FFmpeg; release pipeline runs this test"]
fn video_compatible_preview_and_http_seek_do_not_modify_original() {
    let f = Fixture::new();
    let source = f.movie("clip.mp4", 3);
    f.scan();
    let photo = f
        .library
        .photo(f.photos("videos")[0]["id"].as_i64().unwrap())
        .unwrap();
    let original = fs::read(&source).unwrap();
    let before = fs::metadata(&source).unwrap().modified().unwrap();
    let poster = f.library.thumbs.join(format!("{}.jpg", photo.hash));
    fs::remove_file(&poster).unwrap();
    f.scan();
    assert!(
        !poster.exists(),
        "Scanning should not decode unchanged video again"
    );
    assert!(
        f.library
            .thumbnail(photo.id)
            .unwrap()
            .starts_with(&[0xff, 0xd8])
    );
    assert_eq!(f.library.photo(photo.id).unwrap().hash, photo.hash);
    let url = cleaner_ultra::server::serve(f.library.clone(), 0).unwrap();
    let host = url.trim_start_matches("http://");
    let route = format!("/video/{}", photo.id);
    let (head, body) = video_request(host, "GET", &route, Some("bytes=10-137"));
    assert!(head.starts_with("HTTP/1.1 206"));
    assert_eq!(body, &original[10..138]);
    assert!(head.to_lowercase().contains("content-range: bytes 10-137/"));
    let (head, body) = video_request(host, "HEAD", &route, None);
    assert!(head.starts_with("HTTP/1.1 200"));
    assert!(body.is_empty());
    let (head, _) = video_request(host, "GET", &route, Some("bytes=999999999-"));
    assert!(head.starts_with("HTTP/1.1 416"));
    let preview = f.library.video_source(photo.id, true).unwrap();
    let metadata =
        cleaner_ultra::video::metadata(&preview, &f.library.data.join("scratch")).unwrap();
    assert_eq!(metadata.codec, "vp8");
    assert!((metadata.duration - photo.duration).abs() < 0.25);
    assert!(metadata.width <= 1280);
    let preview_bytes = fs::read(&preview).unwrap();
    let (head, body) = video_request(
        host,
        "GET",
        &format!("{route}?compatible=1"),
        Some("bytes=-128"),
    );
    assert!(head.starts_with("HTTP/1.1 206"));
    assert_eq!(body, &preview_bytes[preview_bytes.len() - 128..]);
    assert_eq!(f.library.video_source(photo.id, true).unwrap(), preview);
    assert_eq!(fs::read(&source).unwrap(), original);
    assert_eq!(fs::metadata(&source).unwrap().modified().unwrap(), before);
    assert!(
        fs::read_dir(&f.library.thumbs).unwrap().all(|e| e
            .unwrap()
            .path()
            .extension()
            .and_then(|s| s.to_str())
            != Some("part"))
    );
}
#[test]
fn folder_filter_and_pending_view_keep_library_counts() {
    let f = Fixture::new();
    f.image("first.jpg", 1);
    f.scan();
    let second = f._temp.path().join("second-root");
    fs::create_dir(&second).unwrap();
    let source = f.image("source.jpg", 2);
    fs::rename(source, second.join("second.jpg")).unwrap();
    f.library.start(second.to_str().unwrap(), false).unwrap();
    f.wait();
    assert_eq!(f.library.progress.lock().errors, 0);
    let all = f.photos("all");
    let root1 = all.iter().find(|p| p["name"] == "first.jpg").unwrap()["root"]
        .as_str()
        .unwrap();
    let root2 = all.iter().find(|p| p["name"] == "second.jpg").unwrap()["root"]
        .as_str()
        .unwrap();
    let first = f
        .library
        .listing_filtered("all", "", 0, "name", &root1)
        .unwrap();
    let other = f
        .library
        .listing_filtered("pending", "", 0, "name", &root2)
        .unwrap();
    assert_eq!(first["total"], 1);
    assert_eq!(other["total"], 1);
    assert_eq!(first["counts"]["all"], 2);
    assert_eq!(other["counts"]["pending"], 2);
    assert_eq!(
        f.library
            .listing_filtered("all", "", 0, "name", "'")
            .unwrap()["total"],
        0
    );
    assert_eq!(f.library.listing("all", "", 0, "name").unwrap()["total"], 2);
}
#[test]
#[ignore = "Requires bundled FFmpeg; release pipeline runs this test"]
fn video_corrupt_input_and_pause_resume_preserve_index() {
    let f = Fixture::new();
    f.movie("clip.mp4", 3);
    f.scan();
    let id = f.photos("videos")[0]["id"].as_i64().unwrap();
    f.library.start(f.root.to_str().unwrap(), false).unwrap();
    f.library.pause();
    f.wait();
    assert!(f.library.progress.lock().paused);
    f.library.start("", true).unwrap();
    f.wait();
    assert_eq!(f.photos("videos")[0]["id"], id);
    fs::write(f.root.join("broken.mov"), b"invalid container").unwrap();
    f.library.start(f.root.to_str().unwrap(), false).unwrap();
    f.wait();
    assert_eq!(f.library.progress.lock().errors, 1);
    assert_eq!(f.photos("videos").len(), 1);
    assert!(
        fs::read_dir(f.library.data.join("scratch"))
            .unwrap()
            .next()
            .is_none()
    );
    let destination = f.library.data.join("cancelled.webm");
    assert!(
        cleaner_ultra::video::transcode(
            &f.root.join("clip.mp4"),
            &destination,
            1024 * 1024,
            || true
        )
        .is_err()
    );
    assert!(f.root.join("clip.mp4").exists());
}

#[test]
#[ignore = "Requires bundled FFmpeg; release pipeline runs this test"]
fn video_slow_streams_leave_control_api_responsive() {
    let f = Fixture::new();
    let source = f.movie("clip.mp4", 3);
    f.scan();
    let id = f.photos("videos")[0]["id"].as_i64().unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap()
        .set_len(1024 * 1024 * 1024)
        .unwrap();
    let meta = fs::metadata(&source).unwrap();
    f.db()
        .execute(
            "UPDATE photos SET size=?,mtime=? WHERE id=?",
            params![
                meta.len() as i64,
                cleaner_ultra::library::modified(&meta),
                id
            ],
        )
        .unwrap();
    let url = cleaner_ultra::server::serve(f.library.clone(), 0).unwrap();
    let host = url.trim_start_matches("http://");
    let mut clients = Vec::new();
    for _ in 0..6 {
        let mut stream = std::net::TcpStream::connect(host).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(stream, "GET /video/{id} HTTP/1.1\r\nHost: {host}\r\n\r\n").unwrap();
        let mut first = [0u8; 1024];
        let count = stream.read(&mut first).unwrap();
        assert!(String::from_utf8_lossy(&first[..count]).starts_with("HTTP/1.1 200"));
        clients.push(stream);
    }
    let started = Instant::now();
    let (header, body) = video_request(host, "GET", "/api/status", None);
    assert!(header.starts_with("HTTP/1.1 200"));
    assert!(started.elapsed() < Duration::from_secs(5));
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["engine"], "Rust");
    drop(clients);
    thread::sleep(Duration::from_millis(100));
}
#[test]
fn duplicates_and_incremental_scan() {
    let f = Fixture::new();
    let one = f.image("one.jpg", 0);
    fs::copy(&one, f.root.join("copy.jpg")).unwrap();
    f.scan();
    assert_eq!(f.photos("duplicates").len(), 2);
    let p = f.photos("all")[0].clone();
    let thumb = f
        .library
        .thumbs
        .join(format!("{}.jpg", p["hash"].as_str().unwrap()));
    let before = fs::metadata(&thumb).unwrap().modified().unwrap();
    f.scan();
    assert_eq!(before, fs::metadata(thumb).unwrap().modified().unwrap());
}
#[test]
fn trash_restore_and_undo_persist() {
    let f = Fixture::new();
    let path = f.image("one.jpg", 0);
    f.scan();
    let id = f.photos("all")[0]["id"].as_i64().unwrap();
    assert!(
        f.library
            .move_photos(&[id], false)
            .unwrap()
            .errors
            .is_empty()
    );
    assert!(!path.exists());
    assert_eq!(f.photos("trash").len(), 1);
    f.library.close();
    let reopened = Library::open(&f.library.data).unwrap();
    assert!(reopened.undo().unwrap().errors.is_empty());
    assert!(path.exists());
    reopened.close();
}
#[test]
fn conflicts_never_overwrite() {
    let f = Fixture::new();
    let path = f.image("one.jpg", 0);
    f.scan();
    let id = f.photos("all")[0]["id"].as_i64().unwrap();
    f.library.move_photos(&[id], false).unwrap();
    fs::write(&path, b"replacement").unwrap();
    assert_eq!(f.library.move_photos(&[id], true).unwrap().errors.len(), 1);
    assert_eq!(fs::read(path).unwrap(), b"replacement");
    assert!(f.library.photo(id).unwrap().trash.is_some());
}
#[test]
fn changed_file_is_protected() {
    let f = Fixture::new();
    let path = f.image("one.jpg", 0);
    f.scan();
    let id = f.photos("all")[0]["id"].as_i64().unwrap();
    fs::write(&path, b"edited").unwrap();
    assert_eq!(f.library.move_photos(&[id], false).unwrap().errors.len(), 1);
    assert!(path.exists());
}
#[test]
fn cull_decisions_and_undo() {
    let f = Fixture::new();
    f.image("one.jpg", 0);
    f.image("two.jpg", 0);
    f.scan();
    let ids: Vec<_> = f
        .photos("all")
        .iter()
        .map(|p| p["id"].as_i64().unwrap())
        .collect();
    f.library.choose(&ids, Some(ids[0]), true).unwrap();
    assert_eq!(f.photos("favorites").len(), 1);
    assert_eq!(f.photos("trash").len(), 1);
    f.library.undo().unwrap();
    assert_eq!(f.photos("all").len(), 2);
    assert_eq!(f.photos("favorites").len(), 0);
    assert_eq!(f.photos("trash").len(), 0);
}
#[test]
fn duplicate_rules_keep_favorites() {
    let f = Fixture::new();
    let source = f.image("one.jpg", 0);
    fs::copy(&source, f.root.join("copy.jpg")).unwrap();
    f.scan();
    let favorite = f.photos("all")[0]["id"].as_i64().unwrap();
    f.library.favorite(&[favorite], true).unwrap();
    let plan = f.library.duplicate_plan("").unwrap();
    assert_eq!(plan["keep"][0]["id"], favorite);
    let remove: Vec<_> = plan["remove"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_i64().unwrap())
        .collect();
    f.library.apply_duplicates(&remove).unwrap();
    assert_eq!(f.photos("all").len(), 1);
    assert_eq!(f.photos("favorites").len(), 1);
}
#[test]
fn duplicate_plan_cannot_remove_all_copies() {
    let f = Fixture::new();
    let source = f.image("one.jpg", 0);
    fs::copy(&source, f.root.join("copy.jpg")).unwrap();
    f.scan();
    let ids: Vec<_> = f
        .photos("all")
        .iter()
        .map(|p| p["id"].as_i64().unwrap())
        .collect();
    assert_eq!(f.library.apply_duplicates(&ids).unwrap().errors.len(), 2);
    assert_eq!(f.photos("all").len(), 2);
}
#[test]
fn export_is_verified_and_exclusive() {
    let f = Fixture::new();
    let source = f.image("one.jpg", 0);
    f.scan();
    let id = f.photos("all")[0]["id"].as_i64().unwrap();
    let out = f._temp.path().join("export");
    f.library
        .export(&[id], out.to_str().unwrap(), false)
        .unwrap();
    assert_eq!(
        analysis::fingerprint(&source).unwrap(),
        analysis::fingerprint(&out.join("one.jpg")).unwrap()
    );
    fs::write(out.join("one.jpg"), b"existing").unwrap();
    f.library
        .export(&[id], out.to_str().unwrap(), false)
        .unwrap();
    assert_eq!(fs::read(out.join("one.jpg")).unwrap(), b"existing");
    assert!(source.exists());
}
#[test]
fn export_hash_failure_leaves_no_published_or_partial_file() {
    let f = Fixture::new();
    let source = f.image("one.jpg", 0);
    f.scan();
    let id = f.photos("all")[0]["id"].as_i64().unwrap();
    fs::write(&source, b"changed after indexing").unwrap();
    let out = f._temp.path().join("export");
    let result = f
        .library
        .export(&[id], out.to_str().unwrap(), false)
        .unwrap();
    assert_eq!(result["done"], 0);
    assert_eq!(result["errors"].as_array().unwrap().len(), 1);
    assert!(!out.join("one.jpg").exists());
    assert_eq!(fs::read_dir(&out).unwrap().count(), 0);
    assert_eq!(fs::read(source).unwrap(), b"changed after indexing");
}
#[test]
fn export_preserves_both_occupied_names() {
    let f = Fixture::new();
    f.image("one.jpg", 0);
    f.scan();
    let id = f.photos("all")[0]["id"].as_i64().unwrap();
    let out = f._temp.path().join("export");
    fs::create_dir(&out).unwrap();
    fs::write(out.join("one.jpg"), b"first").unwrap();
    fs::write(out.join(format!("one_{id}.jpg")), b"second").unwrap();
    let result = f
        .library
        .export(&[id], out.to_str().unwrap(), false)
        .unwrap();
    assert_eq!(result["done"], 0);
    assert_eq!(result["errors"].as_array().unwrap().len(), 1);
    assert_eq!(fs::read(out.join("one.jpg")).unwrap(), b"first");
    assert_eq!(
        fs::read(out.join(format!("one_{id}.jpg"))).unwrap(),
        b"second"
    );
    assert_eq!(fs::read_dir(&out).unwrap().count(), 2);
}
#[test]
fn paused_queue_survives_restart() {
    let f = Fixture::new();
    for i in 0..60 {
        f.image(&format!("{i:03}.jpg"), i);
    }
    f.library.start(f.root.to_str().unwrap(), false).unwrap();
    f.library.pause();
    f.wait();
    assert!(f.library.progress.lock().paused);
    f.library.close();
    let next = Library::open(&f.library.data).unwrap();
    assert!(next.progress.lock().paused);
    next.start("", true).unwrap();
    let start = Instant::now();
    while next.progress.lock().running {
        assert!(start.elapsed() < Duration::from_secs(90));
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(next.listing("all", "", 0, "name").unwrap()["total"], 60);
    next.close();
}
#[test]
fn scan_batches_keep_counts_and_skip_unchanged_grouping() {
    let f = Fixture::new();
    for i in 0..20 {
        f.image(&format!("{i:03}.jpg"), i);
    }
    f.scan();
    let db = f.db();
    let first: (i64, i64, i64) = db
        .query_row(
            "SELECT done,total,errors FROM jobs ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(first, (20, 20, 0));
    let done_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM scan_queue WHERE state='done'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(done_rows, 0);
    let id: i64 = db
        .query_row("SELECT id FROM photos LIMIT 1", [], |r| r.get(0))
        .unwrap();
    db.execute("UPDATE photos SET similar=NULL WHERE id=?", [id])
        .unwrap();
    drop(db);
    f.scan();
    let db = f.db();
    let second: (i64, i64, i64, i64) = db
        .query_row(
            "SELECT done,total,errors,changed FROM jobs ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(second, (20, 20, 0, 0));
    let similar: Option<i64> = db
        .query_row("SELECT similar FROM photos WHERE id=?", [id], |r| r.get(0))
        .unwrap();
    assert_eq!(similar, None);
    let index: Vec<String> = db
        .prepare("PRAGMA index_info(queue_pending_path)")
        .unwrap()
        .query_map([], |r| r.get(2))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(index, ["job", "state", "path"]);
}

#[test]
fn restored_missing_file_rejoins_groups_without_metadata_change() {
    let f = Fixture::new();
    let original = f.image("returned.jpg", 9);
    f.scan();
    let parked = f._temp.path().join("returned-parked.jpg");
    fs::rename(&original, &parked).unwrap();
    f.scan();
    let db = f.db();
    let missing: i64 = db
        .query_row(
            "SELECT missing FROM photos WHERE name='returned.jpg'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(missing, 1);
    drop(db);
    fs::rename(&parked, &original).unwrap();
    f.scan();
    let db = f.db();
    let restored: (i64, Option<i64>) = db
        .query_row(
            "SELECT missing,similar FROM photos WHERE name='returned.jpg'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let changed: i64 = db
        .query_row(
            "SELECT changed FROM jobs ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(restored.0, 0);
    assert!(restored.1.is_some());
    assert_eq!(changed, 1);
}

#[test]
fn retry_scan_errors_processes_only_selected_files() {
    let f = Fixture::new();
    let good = f.image("good.jpg", 1);
    let bad = f.root.join("bad.jpg");
    fs::write(&bad, b"not an image").unwrap();
    f.library.start(f.root.to_str().unwrap(), false).unwrap();
    f.wait();
    let errors = f.library.scan_errors(100, 0).unwrap();
    assert_eq!(errors["total"], 1);
    assert_eq!(
        f.library.scan_errors(1, 1).unwrap()["errors"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let failed_path = errors["errors"][0]["path"].as_str().unwrap().to_string();
    assert_eq!(PathBuf::from(&failed_path).file_name().unwrap(), "bad.jpg");
    assert!(good.exists());
    f.image("bad.jpg", 2);
    f.db()
        .execute(
            "UPDATE jobs SET state='paused' WHERE id=?",
            [errors["job"].as_i64().unwrap()],
        )
        .unwrap();
    assert!(f.library.retry_scan_errors(&[failed_path.clone()]).is_err());
    f.db()
        .execute(
            "UPDATE jobs SET state='complete' WHERE id=?",
            [errors["job"].as_i64().unwrap()],
        )
        .unwrap();
    let retry = f.library.retry_scan_errors(&[failed_path]).unwrap();
    assert_eq!(retry["queued"], 1);
    f.wait();
    assert_eq!(f.library.scan_errors(100, 0).unwrap()["total"], 0);
    let db = f.db();
    let counts: (i64, i64, i64) = db
        .query_row(
            "SELECT done,total,errors FROM jobs ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(counts, (2, 2, 0));
    assert_eq!(f.photos("all").len(), 2);
}

#[test]
fn scan_errors_are_paginated_without_losing_paths() {
    let f = Fixture::new();
    f.image("good.jpg", 1);
    fs::write(f.root.join("bad-a.jpg"), b"invalid a").unwrap();
    fs::write(f.root.join("bad-b.jpg"), b"invalid b").unwrap();
    f.library.start(f.root.to_str().unwrap(), false).unwrap();
    f.wait();
    let first = f.library.scan_errors(1, 0).unwrap();
    let second = f.library.scan_errors(1, 1).unwrap();
    assert_eq!(first["total"], 2);
    assert_eq!(first["errors"].as_array().unwrap().len(), 1);
    assert_eq!(second["errors"].as_array().unwrap().len(), 1);
    assert_ne!(first["errors"][0]["path"], second["errors"][0]["path"]);
    assert_eq!(first["retry_allowed"], true);
}

#[test]
fn old_scan_errors_cannot_replay_an_obsolete_generation() {
    let f = Fixture::new();
    f.image("good.jpg", 1);
    let bad = f.root.join("bad.jpg");
    fs::write(&bad, b"not an image").unwrap();
    f.library.start(f.root.to_str().unwrap(), false).unwrap();
    f.wait();
    let old_path = f.library.scan_errors(10, 0).unwrap()["errors"][0]["path"]
        .as_str()
        .unwrap()
        .to_string();
    fs::remove_file(bad).unwrap();
    f.scan();
    assert_eq!(f.library.scan_errors(10, 0).unwrap()["total"], 0);
    assert!(f.library.retry_scan_errors(&[old_path]).is_err());
    assert_eq!(f.photos("all").len(), 1);
}

#[test]
fn search_matches_cyrillic_names_independent_of_case() {
    let f = Fixture::new();
    f.image("Семья.jpg", 3);
    f.scan();
    assert_eq!(
        f.library.listing("all", "СЕМЬЯ", 0, "name").unwrap()["total"],
        1
    );
    assert_eq!(
        f.library.listing("all", "семья", 0, "name").unwrap()["total"],
        1
    );
    f.db()
        .execute("UPDATE photos SET search_name=''", [])
        .unwrap();
    f.library.close();
    let reopened = Library::open(&f.library.data).unwrap();
    assert_eq!(
        reopened.listing("all", "СеМьЯ", 0, "name").unwrap()["total"],
        1
    );
    reopened.close();
}
#[test]
fn crash_recovery_preserves_source_or_target() {
    let f = Fixture::new();
    let source = f.image("one.jpg", 0);
    f.scan();
    let id = f.photos("all")[0]["id"].as_i64().unwrap();
    let trash = f.root.join(".cleaner_ultra-trash/recover.jpg");
    fs::create_dir_all(trash.parent().unwrap()).unwrap();
    f.db()
        .execute(
            "INSERT INTO operations(photo,source,destination,kind) VALUES(?,?,?,'trash')",
            params![id, source.to_string_lossy(), trash.to_string_lossy()],
        )
        .unwrap();
    fs::hard_link(&source, &trash).unwrap();
    f.library.recover().unwrap();
    assert!(source.exists());
    assert!(!trash.exists());
    f.db()
        .execute(
            "INSERT INTO operations(photo,source,destination,kind) VALUES(?,?,?,'trash')",
            params![id, source.to_string_lossy(), trash.to_string_lossy()],
        )
        .unwrap();
    fs::rename(&source, &trash).unwrap();
    f.library.recover().unwrap();
    assert!(f.library.photo(id).unwrap().trash.is_some());
    assert!(
        f.library
            .move_photos(&[id], true)
            .unwrap()
            .errors
            .is_empty()
    );
}
#[test]
fn recovery_preserves_valid_copy_when_source_was_replaced() {
    let f = Fixture::new();
    let source = f.image("one.jpg", 0);
    f.scan();
    let photo = f
        .library
        .photo(f.photos("all")[0]["id"].as_i64().unwrap())
        .unwrap();
    let target = f.root.join(".cleaner_ultra-trash/recover.jpg");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::copy(&source, &target).unwrap();
    f.db()
        .execute(
            "INSERT INTO operations(photo,source,destination,kind) VALUES(?,?,?,'trash')",
            params![photo.id, source.to_string_lossy(), target.to_string_lossy()],
        )
        .unwrap();
    fs::write(&source, b"new content at original path").unwrap();
    for _ in 0..2 {
        f.library.recover().unwrap();
        assert_eq!(analysis::fingerprint(&target).unwrap(), photo.hash);
        assert_eq!(fs::read(&source).unwrap(), b"new content at original path");
        let state: String = f
            .db()
            .query_row("SELECT state FROM operations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(state, "pending");
    }
    assert_eq!(
        f.library
            .move_photos(&[photo.id], false)
            .unwrap()
            .errors
            .len(),
        1
    );
    f.library.close();
    let reopened = Library::open(&f.library.data).unwrap();
    assert_eq!(analysis::fingerprint(&target).unwrap(), photo.hash);
    reopened.close();
}
#[test]
fn recovery_continues_past_corrupt_copy_without_deleting_it() {
    let f = Fixture::new();
    let first = f.image("first.jpg", 0);
    let second = f.image("second.jpg", 1);
    f.scan();
    let first_id = f
        .photos("all")
        .iter()
        .find(|p| p["name"] == "first.jpg")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let second_id = f
        .photos("all")
        .iter()
        .find(|p| p["name"] == "second.jpg")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let trash = f.root.join(".cleaner_ultra-trash");
    fs::create_dir_all(&trash).unwrap();
    let corrupt = trash.join("corrupt.jpg");
    fs::write(&corrupt, b"not the indexed photo").unwrap();
    let valid = trash.join("valid.jpg");
    fs::rename(&second, &valid).unwrap();
    for (id, source, target) in [(first_id, &first, &corrupt), (second_id, &second, &valid)] {
        f.db()
            .execute(
                "INSERT INTO operations(photo,source,destination,kind) VALUES(?,?,?,'trash')",
                params![id, source.to_string_lossy(), target.to_string_lossy()],
            )
            .unwrap();
    }
    f.library.recover().unwrap();
    assert!(first.exists());
    assert_eq!(fs::read(&corrupt).unwrap(), b"not the indexed photo");
    assert!(!second.exists());
    assert_eq!(
        f.library.photo(second_id).unwrap().trash.as_deref(),
        valid.to_str()
    );
    let states: Vec<String> = f
        .db()
        .prepare("SELECT state FROM operations ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(states, ["pending", "done"]);
}
#[test]
fn interrupted_restore_never_overwrites_replacement() {
    let f = Fixture::new();
    let original = f.image("one.jpg", 0);
    f.scan();
    let id = f.photos("all")[0]["id"].as_i64().unwrap();
    assert!(
        f.library
            .move_photos(&[id], false)
            .unwrap()
            .errors
            .is_empty()
    );
    let photo = f.library.photo(id).unwrap();
    let trash = PathBuf::from(photo.trash.as_ref().unwrap());
    f.db()
        .execute(
            "INSERT INTO operations(photo,source,destination,kind) VALUES(?,?,?,'restore')",
            params![id, trash.to_string_lossy(), original.to_string_lossy()],
        )
        .unwrap();
    fs::write(&original, b"different photo at original path").unwrap();
    f.library.recover().unwrap();
    assert_eq!(analysis::fingerprint(&trash).unwrap(), photo.hash);
    assert_eq!(
        fs::read(&original).unwrap(),
        b"different photo at original path"
    );
    assert_eq!(f.library.move_photos(&[id], true).unwrap().errors.len(), 1);
    let state: String = f
        .db()
        .query_row(
            "SELECT state FROM operations WHERE kind='restore'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "pending");
}
#[test]
fn raw_jpeg_pair_selection() {
    let f = Fixture::new();
    let jpeg = f.image("capture.jpg", 0);
    f.scan();
    let p = f
        .library
        .photo(f.photos("all")[0]["id"].as_i64().unwrap())
        .unwrap();
    let raw = f.root.join("capture.dng");
    fs::copy(&jpeg, &raw).unwrap();
    f.db().execute("INSERT INTO photos(path,root,name,size,mtime,width,height,hash,dhash,sharpness,asset_group) VALUES(?,?,?,?,?,?,?,?,?,?,?)",params![raw.to_string_lossy(),p.root,"capture.dng",p.size,p.mtime,p.width,p.height,p.hash,p.dhash,p.sharpness,p.asset_group]).unwrap();
    f.library.favorite(&[p.id], true).unwrap();
    assert_eq!(f.photos("favorites").len(), 2);
    assert_eq!(f.library.move_photos(&[p.id], false).unwrap().done.len(), 2);
    assert!(!jpeg.exists() && !raw.exists());
    f.library.undo().unwrap();
    assert!(jpeg.exists() && raw.exists());
}

#[test]
fn duplicate_apply_moves_only_confirmed_ids_even_with_raw_pair() {
    let f = Fixture::new();
    let jpeg = f.image("capture.jpg", 0);
    let duplicate = f.root.join("duplicate.jpg");
    fs::copy(&jpeg, &duplicate).unwrap();
    f.scan();
    let jpeg_id = f
        .photos("all")
        .iter()
        .find(|p| p["name"] == "capture.jpg")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let p = f.library.photo(jpeg_id).unwrap();
    let raw = f.root.join("capture.dng");
    fs::copy(&jpeg, &raw).unwrap();
    f.db().execute("INSERT INTO photos(path,root,name,size,mtime,width,height,hash,dhash,sharpness,asset_group) VALUES(?,?,?,?,?,?,?,?,?,?,?)",params![raw.to_string_lossy(),p.root,"capture.dng",p.size,p.mtime,p.width,p.height,p.hash,p.dhash,p.sharpness,p.asset_group]).unwrap();
    let outcome = f.library.apply_duplicates(&[jpeg_id]).unwrap();
    assert_eq!(outcome.done, [jpeg_id]);
    assert!(!jpeg.exists());
    assert!(raw.exists());
    assert!(duplicate.exists());
}
#[test]
fn settings_and_review_persist() {
    let f = Fixture::new();
    f.image("one.jpg", 0);
    f.scan();
    let id = f.photos("all")[0]["id"].as_i64().unwrap();
    let mut settings = f.library.settings();
    settings.theme = "dark".into();
    settings.sensitivity = 4;
    settings.ui_state = serde_json::json!({"view":"favorites","offset":80});
    f.library.save_settings(settings).unwrap();
    f.library.decision(&[id], "skip").unwrap();
    f.library.close();
    let next = Library::open(&f.library.data).unwrap();
    assert_eq!(next.settings().theme, "dark");
    assert_eq!(next.photo(id).unwrap().decision, "skip");
    next.close();
}
#[test]
fn metric_search_matches_bruteforce() {
    let mut tree = BKTree::default();
    for value in [0u64, 7, 15, 255, 65535] {
        tree.add(value, value as i64);
    }
    let mut found = tree.find(0, 3);
    found.sort();
    assert_eq!(found, vec![0, 7]);
}
#[test]
fn segmented_hamming_search_matches_bruteforce() {
    let values: Vec<u64> = (0..3000u64)
        .map(|i| i.wrapping_mul(0x9e3779b97f4a7c15).rotate_left(17))
        .collect();
    for radius in [0, 2, 6, 12] {
        let mut index = HammingIndex::new(radius);
        for (id, &hash) in values.iter().enumerate() {
            index.add(hash, id as i64);
        }
        index.add(values[0], 3000);
        for &base in values.iter().step_by(200) {
            for query in [base, base ^ 0b101010, base ^ 0xfff] {
                let mut expected: Vec<i64> = values
                    .iter()
                    .enumerate()
                    .filter(|(_, hash)| (**hash ^ query).count_ones() <= radius)
                    .map(|(id, _)| id as i64)
                    .collect();
                if (values[0] ^ query).count_ones() <= radius {
                    expected.push(3000);
                }
                let mut actual = index.find(query);
                actual.sort();
                assert_eq!(actual, expected, "radius={radius}");
            }
        }
    }
}
#[test]
fn burst_groups_and_child_scan() {
    let f = Fixture::new();
    f.image("a/one.jpg", 0);
    f.image("b/one.jpg", 0);
    f.image("a/two.jpg", 0);
    f.scan();
    for p in f.photos("all") {
        let time = if p["path"].as_str().unwrap().contains("two") {
            102.0
        } else {
            100.0
        };
        f.db()
            .execute(
                "UPDATE photos SET captured=? WHERE id=?",
                params![time, p["id"].as_i64().unwrap()],
            )
            .unwrap();
    }
    f.library.group().unwrap();
    assert_eq!(f.photos("bursts").len(), 2);
    f.library
        .start(f.root.join("a").to_str().unwrap(), false)
        .unwrap();
    f.wait();
    assert_eq!(f.photos("all").len(), 3);
}
#[test]
fn api_rejects_cross_site_mutations() {
    let f = Fixture::new();
    let url = cleaner_ultra::server::serve(f.library.clone(), 0).unwrap();
    let host = url.trim_start_matches("http://");
    fn request(host: &str, data: &str) -> String {
        let mut stream = std::net::TcpStream::connect(host).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.write_all(data.as_bytes()).unwrap();
        let (head, body) = common::response(&mut stream);
        let response = format!("{head}\r\n\r\n{}", String::from_utf8(body).unwrap());
        response
    }
    let response = request(
        host,
        &format!(
            "POST /api/undo HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Length: 2\r\n\r\n{{}}"
        ),
    );
    assert!(response.contains("403"));
    let response = request(
        host,
        "GET /api/session HTTP/1.1\r\nHost: evil.example\r\nConnection: close\r\n\r\n",
    );
    assert!(response.contains("403"));
    f.library.close();
    thread::sleep(Duration::from_millis(250));
}

#[test]
#[ignore = "Run explicitly for 100,000-row scale validation"]
fn metadata_benchmark_100k() {
    let f = Fixture::new();
    let mut db = f.db();
    let tx = db.transaction().unwrap();
    for i in 0..100000i64 {
        let hash = format!(
            "{:016x}",
            ((i as u64).wrapping_mul(0x9e3779b97f4a7c15)).rotate_left(17)
        );
        tx.execute("INSERT INTO photos(path,root,name,size,mtime,width,height,hash,dhash,ahash,crop_hash,sharpness,asset_group) VALUES(?,?,?,1,1,1200,800,?,?,?,?,1,?)",params![format!("/benchmark/{i}.jpg"),"/benchmark",format!("{i}.jpg"),format!("{i}"),hash,hash,hash,format!("{i}")]).unwrap();
    }
    tx.commit().unwrap();
    let started = Instant::now();
    f.library.group().unwrap();
    let grouped = started.elapsed();
    let started = Instant::now();
    let page = f.library.listing("all", "", 0, "name").unwrap();
    assert_eq!(page["total"], 100000);
    println!(
        "100k metadata: grouping={grouped:?}, listing={:?}",
        started.elapsed()
    );
}

#[test]
fn legacy_v01_database_preserves_favorites_and_restores_trash() {
    let f = Fixture::new();
    let original = f.image("legacy.jpg", 5);
    let original = fs::canonicalize(original).unwrap();
    let original = PathBuf::from(original.to_string_lossy().trim_start_matches(r"\\?\"));
    let legacy_root = fs::canonicalize(&f.root).unwrap();
    let legacy_root = PathBuf::from(legacy_root.to_string_lossy().trim_start_matches(r"\\?\"));
    let hash = analysis::fingerprint(&original).unwrap();
    let trash_dir = f.root.join(".cleaner_ultra-trash");
    fs::create_dir(&trash_dir).unwrap();
    let trash = trash_dir.join("legacy-copy.jpg");
    fs::copy(&original, &trash).unwrap();
    let data = f._temp.path().join("legacy-index");
    fs::create_dir(&data).unwrap();
    let db = Connection::open(data.join("library.sqlite3")).unwrap();
    db.execute_batch("CREATE TABLE photos(id INTEGER PRIMARY KEY,path TEXT UNIQUE NOT NULL,root TEXT NOT NULL,name TEXT,size INTEGER,mtime INTEGER,width INTEGER,height INTEGER,captured REAL,hash TEXT,dhash TEXT,sharpness REAL,favorite INTEGER DEFAULT 0,trash TEXT,missing INTEGER DEFAULT 0,similar INTEGER,burst INTEGER,seen TEXT);CREATE TABLE operations(id INTEGER PRIMARY KEY,photo INTEGER,source TEXT,destination TEXT,kind TEXT,state TEXT DEFAULT 'pending');").unwrap();
    db.execute("INSERT INTO photos(id,path,root,name,size,mtime,width,height,hash,dhash,sharpness,favorite) VALUES(41,?,?,?,1,1,160,120,?,'0000000000000000',1,1)",params![original.to_str().unwrap(),legacy_root.to_str().unwrap(),"legacy.jpg",hash]).unwrap();
    let restored = legacy_root.join("legacy-copy.jpg");
    db.execute("INSERT INTO photos(id,path,root,name,size,mtime,width,height,hash,dhash,sharpness,trash) VALUES(42,?,?,?,1,1,160,120,?,'0000000000000000',1,?)",params![restored.to_str().unwrap(),legacy_root.to_str().unwrap(),"legacy-copy.jpg",hash,trash.to_str().unwrap()]).unwrap();
    drop(db);
    let migrated = Library::open(&data).unwrap();
    assert!(migrated.photo(41).unwrap().favorite);
    assert_eq!(migrated.move_photos(&[42], true).unwrap().done, vec![42]);
    assert_eq!(analysis::fingerprint(&restored).unwrap(), hash);
    migrated.start(f.root.to_str().unwrap(), false).unwrap();
    while migrated.progress.lock().running {
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(migrated.progress.lock().errors, 0);
    assert!(migrated.photo(41).unwrap().favorite);
    let db = Connection::open(data.join("library.sqlite3")).unwrap();
    let average: String = db
        .query_row("SELECT ahash FROM photos WHERE id=41", [], |row| row.get(0))
        .unwrap();
    let rows: Vec<(i64, String, String)> = db
        .prepare("SELECT id,path,ahash FROM photos")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(!average.is_empty(), "Migration rows: {rows:?}");
    assert_eq!(rows.len(), 2);
    drop(db);
    migrated.close();
}

#[test]
fn cache_budget_evicts_only_previews() {
    let f = Fixture::new();
    let original = f.image("untouched.jpg", 8);
    let before = analysis::fingerprint(&original).unwrap();
    let note = f.library.thumbs.join("notes.txt");
    fs::write(&note, b"preserve non-preview files").unwrap();
    for i in 0..3 {
        let file = fs::File::create(f.library.thumbs.join(format!("{:064x}.jpg", i + 1))).unwrap();
        file.set_len(16 * 1024 * 1024).unwrap();
    }
    let mut settings = f.library.settings();
    settings.cache_mb = 32;
    f.library.save_settings(settings).unwrap();
    let size: u64 = fs::read_dir(&f.library.thumbs)
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum();
    assert!(size <= 32 * 1024 * 1024 + fs::metadata(&note).unwrap().len());
    assert_eq!(analysis::fingerprint(&original).unwrap(), before);
    assert!(note.exists());
}

#[test]
fn cache_remains_bounded_during_preview_regeneration() {
    let f = Fixture::new();
    f.image("regenerate.jpg", 9);
    f.scan();
    let photo = f
        .library
        .photo(f.photos("all")[0]["id"].as_i64().unwrap())
        .unwrap();
    let mut settings = f.library.settings();
    settings.cache_mb = 32;
    f.library.save_settings(settings).unwrap();
    for i in 0..3 {
        let file = fs::File::create(f.library.thumbs.join(format!("{:064x}.jpg", i + 1))).unwrap();
        file.set_len(16 * 1024 * 1024).unwrap();
    }
    let thumbnail = f.library.thumbs.join(format!("{}.jpg", photo.hash));
    for _ in 0..64 {
        if thumbnail.exists() {
            fs::remove_file(&thumbnail).unwrap();
        }
        let bytes = f.library.thumbnail(photo.id).unwrap();
        assert_eq!(&bytes[..2], b"\xff\xd8");
    }
    let size: u64 = fs::read_dir(&f.library.thumbs)
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum();
    assert!(size <= 32 * 1024 * 1024);
}

#[test]
fn duplicate_plan_exposes_all_files_with_hash_and_size() {
    let f = Fixture::new();
    let original = f.image("original.jpg", 7);
    for index in 0..102 {
        fs::copy(&original, f.root.join(format!("copy-{index:03}.jpg"))).unwrap();
    }
    f.scan();
    let plan = f.library.duplicate_plan("").unwrap();
    let keep = plan["keep"].as_array().unwrap();
    let remove = plan["remove"].as_array().unwrap();
    assert_eq!(keep.len(), 1);
    assert_eq!(remove.len(), 102);
    assert!(keep.iter().chain(remove).all(|item| {
        item["hash"].as_str().is_some_and(|hash| !hash.is_empty())
            && item["size"].as_i64().is_some_and(|size| size > 0)
    }));
}

#[test]
fn culling_group_respects_selected_root() {
    let f = Fixture::new();
    let first = f.image("first.jpg", 8);
    fs::copy(&first, f.root.join("second.jpg")).unwrap();
    f.scan();
    let other_root = f._temp.path().join("other-root");
    fs::create_dir(&other_root).unwrap();
    fs::copy(&first, other_root.join("third.jpg")).unwrap();
    fs::copy(&first, other_root.join("fourth.jpg")).unwrap();
    f.library
        .start(other_root.to_str().unwrap(), false)
        .unwrap();
    f.wait();
    let all = f.photos("all");
    let first_root = all.iter().find(|item| item["name"] == "first.jpg").unwrap()["root"]
        .as_str()
        .unwrap()
        .to_string();
    let other_root = all.iter().find(|item| item["name"] == "third.jpg").unwrap()["root"]
        .as_str()
        .unwrap()
        .to_string();
    let first_group = f
        .library
        .next_group_filtered("duplicates", &first_root)
        .unwrap();
    let other_group = f
        .library
        .next_group_filtered("duplicates", &other_root)
        .unwrap();
    assert_eq!(first_group["photos"].as_array().unwrap().len(), 2);
    assert_eq!(other_group["photos"].as_array().unwrap().len(), 2);
    assert_eq!(first_group["total"], 1);
    assert_eq!(other_group["total"], 1);
    assert_eq!(
        f.library
            .next_group_filtered("duplicates", "missing-root")
            .unwrap()["total"],
        0
    );
}
#[test]
fn old_brand_media_trash_restores_after_rename() {
    let f = Fixture::new();
    let original = f.image("legacy.jpg", 123);
    f.scan();
    let id = f.photos("all")[0]["id"].as_i64().unwrap();
    let vault = f.root.join(".photosort-trash/legacy.jpg");
    fs::create_dir_all(vault.parent().unwrap()).unwrap();
    fs::rename(&original, &vault).unwrap();
    f.db()
        .execute(
            "UPDATE photos SET trash=? WHERE id=?",
            params![vault.to_string_lossy(), id],
        )
        .unwrap();
    let outcome = f.library.move_photos(&[id], true).unwrap();
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    assert!(original.is_file());
    assert!(!vault.exists());
}
