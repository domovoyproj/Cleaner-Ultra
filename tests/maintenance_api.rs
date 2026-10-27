mod common;
use cleaner_ultra::{library::Library, server};
use serde_json::{Value, json};
use std::{io::Write, net::TcpStream, time::Duration};
fn request(host: &str, path: &str, body: Option<Value>, token: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(host).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let request = if let Some(body) = body {
        let body = body.to_string();
        format!(
            "POST {path} HTTP/1.1\r\nHost: {host}\r\nX-CleanerUltra-Token: {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    } else {
        format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
    };
    stream.write_all(request.as_bytes()).unwrap();
    let (head, body) = common::response(&mut stream);
    (
        head.split_whitespace().nth(1).unwrap().parse().unwrap(),
        String::from_utf8(body).unwrap(),
    )
}
#[test]
fn maintenance_routes_require_auth_confirmation_and_server_issued_ids() {
    let t = tempfile::tempdir().unwrap();
    let library = Library::open(t.path()).unwrap();
    let url = server::serve(library.clone(), 0).unwrap();
    let host = url.trim_start_matches("http://");
    let (status, session) = request(host, "/api/session", None, "");
    assert_eq!(status, 200);
    let session: Value = serde_json::from_str(&session).unwrap();
    let token = session["token"].as_str().unwrap();
    assert_eq!(session["version"], env!("CARGO_PKG_VERSION"));
    for path in [
        "/api/maintenance/scan",
        "/api/cleanup/scan",
        "/api/cleanup/stop",
        "/api/cleanup/apply",
        "/api/cleanup/restore",
        "/api/startup/scan",
        "/api/registry/scan",
        "/api/startup/apply",
        "/api/registry/apply",
        "/api/power/apply",
        "/api/maintenance/apply",
        "/api/maintenance/restore",
        "/api/apps/uninstall",
        "/api/space/optimize",
        "/api/optimizer/stop",
    ] {
        assert_eq!(request(host, path, Some(json!({})), "wrong-token").0, 403);
    }
    for path in [
        "/api/maintenance/apply",
        "/api/startup/apply",
        "/api/registry/apply",
        "/api/power/apply",
        "/api/cleanup/apply",
        "/api/cleanup/restore",
        "/api/apps/uninstall",
        "/api/space/optimize",
    ] {
        assert_eq!(request(host, path, Some(json!({})), token).0, 400);
    }
    assert_eq!(
        request(
            host,
            "/api/maintenance/apply",
            Some(json!({"ids":["invented"],"action":"disable","confirmed":true})),
            token
        )
        .0,
        400
    );
    assert_eq!(
        request(
            host,
            "/api/apps/uninstall",
            Some(json!({"id":"invented","confirmed":true})),
            token
        )
        .0,
        400
    );
    assert_eq!(
        request(
            host,
            "/api/space/optimize",
            Some(json!({"drive":"C: & erase","analyze":false,"confirmed":true})),
            token
        )
        .0,
        400
    );
    for path in [
        "/maintenance.js",
        "/maintenance.css",
        "/wizard.js",
        "/cleanup-ui.js",
        "/disk-tree.js",
        "/design09.css",
        "/desktop.css",
        "/api/cleanup/catalog",
        "/api/cleanup/progress",
        "/api/cleanup/files",
        "/api/cleanup/trash",
        "/cleanup-plan.js",
        "/performance.js",
        "/api/cache/status",
        "/api/cleanup/defaults",
        "/api/maintenance/backups",
        "/api/history",
        "/api/optimizer/status",
    ] {
        assert_eq!(request(host, path, None, "").0, 200);
    }
    let (_, html) = request(host, "/", None, "");
    assert!(html.contains("data-page=\"registry\""));
    assert!(html.contains("data-page=\"startup\""));
    assert!(html.contains("Ускорить и очистить"));
    assert!(html.contains("id=\"apps-search\""));
    assert!(html.contains("Дерево диска"));
    assert!(html.contains("Кеш браузеров"));
    assert!(html.contains("Временные файлы Windows"));
    assert!(html.contains("Cleaner Ultra"));
    assert!(!html.contains("PhotoSort"));
    // The preview directory belongs to this fixture; no user data is touched.
    assert_eq!(request(host, "/api/cache", Some(json!({})), token).0, 200);
    let (_, history) = request(host, "/api/history", None, "");
    let history: Value = serde_json::from_str(&history).unwrap();
    assert_eq!(history["items"][0]["kind"], "/api/cache");
    assert_eq!(history["items"][0]["result"]["freed_bytes"], 0);
    library.close();
}
#[cfg(windows)]
#[test]
fn native_inventory_is_read_only_and_usable() {
    let t = tempfile::tempdir().unwrap();
    let library = Library::open(t.path()).unwrap();
    let url = server::serve(library.clone(), 0).unwrap();
    let host = url.trim_start_matches("http://");
    let (_, session) = request(host, "/api/session", None, "");
    let session: Value = serde_json::from_str(&session).unwrap();
    let token = session["token"].as_str().unwrap();
    let (code, apps) = request(host, "/api/apps", None, "");
    assert_eq!(code, 200);
    let apps: Value = serde_json::from_str(&apps).unwrap();
    assert!(apps["items"].is_array());
    let (code, registry) = request(host, "/api/maintenance/scan", Some(json!({})), token);
    assert_eq!(code, 200);
    let registry: Value = serde_json::from_str(&registry).unwrap();
    assert!(registry["items"].is_array());
    for path in ["/api/startup/scan", "/api/registry/scan"] {
        let (code, body) = request(host, path, Some(json!({})), token);
        assert_eq!(code, 200, "{path}: {body}");
        assert!(serde_json::from_str::<Value>(&body).unwrap()["items"].is_array());
    }
    for path in ["/api/performance", "/api/power"] {
        let (code, body) = request(host, path, None, "");
        assert_eq!(code, 200, "{path}: {body}");
    }
    assert_eq!(
        request(
            host,
            "/api/power/apply",
            Some(json!({"id":"invented","confirmed":true})),
            token
        )
        .0,
        400
    );
    // No modification and no deinstaller launch occurs in this test.
    assert_eq!(
        request(
            host,
            "/api/maintenance/apply",
            Some(json!({"confirmed":true,"action":"clean","ids":["not-in-snapshot"]})),
            token
        )
        .0,
        400
    );
    library.close();
}
