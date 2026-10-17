use crate::{
    analysis,
    library::{Library, Settings},
};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    io::{Read, Seek, SeekFrom},
    path::{Component, Path},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

pub fn serve(library: Arc<Library>, port: u16) -> Result<String> {
    let space = crate::space::Space::open(&library.data)?;
    let server =
        Arc::new(Server::http(("127.0.0.1", port)).map_err(|e| anyhow::anyhow!(e.to_string()))?);
    let address = server.server_addr().to_ip().unwrap();
    let host = address.to_string();
    let url = format!("http://{host}");
    let token = uuid::Uuid::new_v4().to_string() + &uuid::Uuid::new_v4().to_string();
    let streams = Arc::new(AtomicUsize::new(0));
    // Bounded request workers prevent full-resolution previews exhausting memory.
    for _ in 0..4 {
        let server = server.clone();
        let library = library.clone();
        let token = token.clone();
        let host = host.clone();
        let streams = streams.clone();
        let space = space.clone();
        thread::spawn(move || {
            loop {
                if library.is_closed() {
                    space.close();
                    break;
                }
                let mut request = match server.recv_timeout(std::time::Duration::from_millis(100)) {
                    Ok(Some(request)) => request,
                    Ok(None) => continue,
                    Err(_) => break,
                };
                let mut allowed = request
                    .headers()
                    .iter()
                    .any(|h| h.field.equiv("Host") && h.value.as_str() == host);
                if *request.method() == Method::Post {
                    allowed &= request
                        .headers()
                        .iter()
                        .any(|h| h.field.equiv("X-PhotoSort-Token") && h.value.as_str() == token)
                }
                if !allowed {
                    respond(
                        request,
                        serde_json::to_vec(&json!({"error":"Доступ запрещён"})).unwrap(),
                        "application/json",
                        403,
                    );
                    continue;
                }
                if request.url().starts_with("/video/") {
                    match video_response(&library, &request) {
                        Ok(response) => {
                            if let Some(slot) = StreamSlot::reserve(&streams) {
                                thread::spawn(move || {
                                    let _slot = slot;
                                    let _ = request.respond(response);
                                });
                            } else {
                                respond(
                                    request,
                                    b"{\"error\":\"Too many video streams\"}".to_vec(),
                                    "application/json",
                                    503,
                                );
                            }
                        }
                        Err(error) => respond(
                            request,
                            serde_json::to_vec(&json!({"error":error.to_string()})).unwrap(),
                            "application/json",
                            400,
                        ),
                    }
                    continue;
                }
                let result = route(&library, &space, &token, &mut request);
                match result {
                    Ok((bytes, mime)) => respond(request, bytes, &mime, 200),
                    Err(error) => respond(
                        request,
                        serde_json::to_vec(&json!({"error":error.to_string()})).unwrap(),
                        "application/json",
                        400,
                    ),
                }
            }
        });
    }
    Ok(url)
}
struct StreamSlot(Arc<AtomicUsize>);
impl StreamSlot {
    fn reserve(streams: &Arc<AtomicUsize>) -> Option<Self> {
        let mut count = streams.load(Ordering::Relaxed);
        while count < 6 {
            match streams.compare_exchange_weak(
                count,
                count + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(Self(streams.clone())),
                Err(current) => count = current,
            }
        }
        None
    }
}
impl Drop for StreamSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
fn video_response(library: &Library, request: &Request) -> Result<Response<Box<dyn Read + Send>>> {
    if !matches!(*request.method(), Method::Get | Method::Head) {
        bail!("Метод не поддерживается");
    }
    let url = url::Url::parse(&format!("http://127.0.0.1{}", request.url()))?;
    let id: i64 = url
        .path()
        .strip_prefix("/video/")
        .context_or("Нет видео")?
        .parse()?;
    let compatible = url
        .query_pairs()
        .any(|(key, value)| key == "compatible" && value == "1");
    // GET never launches an expensive encode or writes new files.
    let photo = library.photo(id)?;
    let path = if compatible {
        if photo.media_kind != "video" || photo.missing {
            bail!("Видео недоступно");
        }
        let path = library.thumbs.join(format!("video-{}.webm", photo.hash));
        if !path.is_file() {
            bail!("Сначала подготовьте совместимое превью");
        }
        path
    } else {
        library.video_source(id, false)?
    };
    let mut file = std::fs::File::open(&path)?;
    let total = file.metadata()?.len();
    let range = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Range"))
        .map(|header| header.value.as_str());
    let selection = byte_range(range, total);
    let mut headers = vec![
        Header::from_bytes("Content-Type", crate::video::mime(&path)).unwrap(),
        Header::from_bytes("Accept-Ranges", "bytes").unwrap(),
        Header::from_bytes("Cache-Control", "no-store").unwrap(),
        Header::from_bytes("X-Content-Type-Options", "nosniff").unwrap(),
    ];
    let (start, length, partial) = match selection {
        Some(selection) => selection,
        None => {
            headers.push(Header::from_bytes("Content-Range", format!("bytes */{total}")).unwrap());
            return Ok(Response::new(
                StatusCode(416),
                headers,
                Box::new(std::io::empty()),
                Some(0),
                None,
            ));
        }
    };
    if partial {
        headers.push(
            Header::from_bytes(
                "Content-Range",
                format!("bytes {start}-{}/{total}", start + length - 1),
            )
            .unwrap(),
        );
    }
    file.seek(SeekFrom::Start(start))?;
    let reader: Box<dyn Read + Send> = Box::new(file.take(length));
    Ok(Response::new(
        StatusCode(if partial { 206 } else { 200 }),
        headers,
        reader,
        Some(usize::try_from(length)?),
        None,
    ))
}
pub fn byte_range(range: Option<&str>, total: u64) -> Option<(u64, u64, bool)> {
    let Some(range) = range else {
        return Some((0, total, false));
    };
    let (start, end) = range.strip_prefix("bytes=")?.split_once('-')?;
    if total == 0 || end.contains(',') {
        return None;
    }
    if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?.min(total);
        return (suffix > 0).then_some((total - suffix, suffix, true));
    }
    let start = start.parse::<u64>().ok()?;
    let end = if end.is_empty() {
        total - 1
    } else {
        end.parse::<u64>().ok()?.min(total - 1)
    };
    (start < total && end >= start).then_some((start, end.saturating_sub(start) + 1, true))
}
fn respond(request: Request, bytes: Vec<u8>, mime: &str, status: u16) {
    let response=Response::from_data(bytes).with_status_code(StatusCode(status)).with_header(Header::from_bytes("Content-Type",mime).unwrap()).with_header(Header::from_bytes("Cache-Control","no-store").unwrap()).with_header(Header::from_bytes("X-Content-Type-Options","nosniff").unwrap()).with_header(Header::from_bytes("Content-Security-Policy","default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self' blob:; frame-ancestors 'none'").unwrap());
    let _ = request.respond(response);
}
fn ids(body: &Value) -> Result<Vec<i64>> {
    let array = body
        .get("ids")
        .and_then(Value::as_array)
        .context_or("Некорректный список фотографий")?;
    if array.len() > 100000 {
        bail!("Слишком много фотографий")
    };
    array
        .iter()
        .map(|v| v.as_i64().context_or("Некорректный идентификатор"))
        .collect()
}
trait ContextOption<T> {
    fn context_or(self, text: &str) -> Result<T>;
}
impl<T> ContextOption<T> for Option<T> {
    fn context_or(self, text: &str) -> Result<T> {
        self.ok_or_else(|| anyhow::anyhow!(text.to_string()))
    }
}
fn route(
    library: &Arc<Library>,
    space: &Arc<crate::space::Space>,
    token: &str,
    request: &mut Request,
) -> Result<(Vec<u8>, String)> {
    let url = url::Url::parse(&format!("http://127.0.0.1{}", request.url()))?;
    let path = url.path();
    let parameters: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    let get = |name: &str, default: &str| parameters.get(name).cloned().unwrap_or(default.into());
    let value = if *request.method() == Method::Get {
        match path {
            "/api/space/status" => {
                let mut info = space.status()?;
                info["theme"] = json!(library.settings().theme);
                info
            }
            "/api/space/progress" => serde_json::to_value(space.progress.lock().clone())?,
            "/api/space/files" => space.files(
                &get("mode", "all"),
                &get("search", ""),
                get("offset", "0").parse().unwrap_or(0),
                &get("folder", ""),
                &get("category", ""),
            )?,
            "/api/space/folders" => {
                space.folders(&get("path", ""), get("offset", "0").parse().unwrap_or(0))?
            }
            "/api/space/trash" => space.trash(get("offset", "0").parse().unwrap_or(0))?,
            "/api/session" => {
                json!({"token":token,"version":env!("CARGO_PKG_VERSION"),"engine":"Rust"})
            }
            "/api/photos" => library.listing_filtered(
                &get("view", "all"),
                &get("search", ""),
                get("offset", "0").parse().unwrap_or(0),
                &get("sort", "date"),
                &get("root", ""),
            )?,
            "/api/progress" => serde_json::to_value(library.progress.lock().clone())?,
            "/api/scan/errors" => library.scan_errors(
                get("limit", "500")
                    .parse::<i64>()
                    .unwrap_or(500)
                    .clamp(1, 1000),
                get("offset", "0").parse::<i64>().unwrap_or(0).max(0),
            )?,
            "/api/status" => library.status()?,
            "/api/group" => {
                library.next_group_filtered(&get("kind", "similar"), &get("root", ""))?
            }
            "/" => {
                return Ok((
                    include_bytes!("../photosort/web/space.html").to_vec(),
                    "text/html; charset=utf-8".into(),
                ));
            }
            "/space.js" => {
                return Ok((
                    include_bytes!("../photosort/web/space.js").to_vec(),
                    "text/javascript; charset=utf-8".into(),
                ));
            }
            "/space.css" => {
                return Ok((
                    include_bytes!("../photosort/web/space.css").to_vec(),
                    "text/css; charset=utf-8".into(),
                ));
            }
            "/photos" => {
                return Ok((
                    include_bytes!("../photosort/web/index.html").to_vec(),
                    "text/html; charset=utf-8".into(),
                ));
            }
            "/app.js" => {
                return Ok((
                    include_bytes!("../photosort/web/app.js").to_vec(),
                    "text/javascript; charset=utf-8".into(),
                ));
            }
            "/icon.ico" => {
                return Ok((
                    include_bytes!("../packaging/photosort.ico").to_vec(),
                    "image/x-icon".into(),
                ));
            }
            "/style.css" => {
                return Ok((
                    include_bytes!("../photosort/web/style.css").to_vec(),
                    "text/css; charset=utf-8".into(),
                ));
            }
            "/update.css" => {
                return Ok((
                    include_bytes!("../photosort/web/update.css").to_vec(),
                    "text/css; charset=utf-8".into(),
                ));
            }
            "/face-worker.js" => {
                return Ok((
                    include_bytes!("../photosort/web/face-worker.js").to_vec(),
                    "text/javascript; charset=utf-8".into(),
                ));
            }
            _ if path.starts_with("/media/") => {
                let id: i64 = path[7..].parse()?;
                let bytes = if parameters.contains_key("full") {
                    let photo = library.photo(id)?;
                    analysis::preview(
                        Path::new(photo.trash.as_deref().unwrap_or(&photo.path)),
                        &library.data.join("scratch"),
                    )?
                } else {
                    library.thumbnail(id)?
                };
                return Ok((bytes, "image/jpeg".into()));
            }
            _ if path.starts_with("/vision/") => {
                let relative = Path::new(&path[8..]);
                if relative
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_)))
                {
                    bail!("Некорректный путь")
                };
                let exe = std::env::current_exe()?;
                let root = exe.parent().unwrap().join("vision");
                let root = if root.exists() {
                    root
                } else {
                    Path::new(".photosort/tools/vision").to_path_buf()
                };
                let mime = match relative.extension().and_then(|e| e.to_str()) {
                    Some("wasm") => "application/wasm",
                    Some("js" | "mjs") => "text/javascript",
                    _ => "application/octet-stream",
                };
                return Ok((std::fs::read(root.join(relative))?, mime.into()));
            }
            _ => bail!("Не найдено"),
        }
    } else if *request.method() == Method::Post {
        let mut bytes = Vec::new();
        request
            .as_reader()
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 4 * 1024 * 1024 {
            bail!("Запрос слишком большой")
        };
        let body: Value = serde_json::from_slice(if bytes.is_empty() { b"{}" } else { &bytes })?;
        match path {
            "/api/space/theme" => {
                let theme = body["theme"].as_str().context_or("Выберите тему")?;
                if !["light", "dark", "system"].contains(&theme) {
                    bail!("Неизвестная тема")
                }
                let mut settings = library.settings();
                settings.theme = theme.into();
                library.save_settings(settings)?;
                json!({"ok":true})
            }
            "/api/space/scan" => space.start(body["path"].as_str().unwrap_or(""))?,
            "/api/space/stop" => {
                space.stop();
                json!({"ok":true})
            }
            "/api/space/trash" => space.quarantine(
                &ids(&body)?,
                body["scan"].as_i64().context_or("Обновите анализ")?,
            )?,
            "/api/space/restore" => space.restore(&ids(&body)?)?,
            "/api/space/reveal" => {
                space.reveal(body["id"].as_i64().context_or("Выберите файл")?)?
            }
            "/api/space/reveal-trash" => {
                space.reveal_trash(body["id"].as_i64().context_or("Выберите файл")?)?
            }
            "/api/space/optimize" => crate::space::optimize_drive(
                body["drive"].as_str().unwrap_or(""),
                body["analyze"].as_bool().unwrap_or(false),
            )?,
            "/api/space/system" => crate::space::system_page(body["page"].as_str().unwrap_or(""))?,
            "/api/scan" => {
                library.start(body["path"].as_str().unwrap_or(""), false)?;
                json!({"ok":true})
            }
            "/api/resume" => {
                library.start("", true)?;
                json!({"ok":true})
            }
            "/api/stop" | "/api/pause" => {
                library.pause();
                json!({"ok":true})
            }
            "/api/scan/retry" => {
                let paths = body
                    .get("paths")
                    .and_then(Value::as_array)
                    .context_or("Некорректный список файлов")?;
                if paths.len() > 10000 {
                    bail!("Слишком много файлов")
                }
                let paths: Vec<String> = paths
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::to_owned)
                            .context_or("Некорректный путь")
                    })
                    .collect::<Result<_>>()?;
                library.retry_scan_errors(&paths)?
            }
            "/api/pick" => {
                let folder = rfd::FileDialog::new()
                    .set_title("Выберите папку")
                    .pick_folder();
                json!({"path":folder.map(|p|p.to_string_lossy().to_string()).unwrap_or_default()})
            }
            "/api/favorite" => {
                library.favorite(&ids(&body)?, body["value"].as_bool().unwrap_or(true))?
            }
            "/api/decision" => {
                library.decision(&ids(&body)?, body["decision"].as_str().unwrap_or("skip"))?
            }
            "/api/trash" => serde_json::to_value(library.move_photos(&ids(&body)?, false)?)?,
            "/api/restore" => serde_json::to_value(library.move_photos(&ids(&body)?, true)?)?,
            "/api/undo" => serde_json::to_value(library.undo()?)?,
            "/api/choose" => serde_json::to_value(library.choose(
                &ids(&body)?,
                body["winner"].as_i64(),
                body["trash_others"].as_bool().unwrap_or(false),
            )?)?,
            "/api/duplicates/plan" => {
                library.duplicate_plan(body["preferred_folder"].as_str().unwrap_or(""))?
            }
            "/api/duplicates/apply" => {
                serde_json::to_value(library.apply_duplicates(&ids(&body)?)?)?
            }
            "/api/export" => library.export(
                &ids(&body)?,
                body["path"].as_str().unwrap_or(""),
                body["structure"].as_bool().unwrap_or(true),
            )?,
            "/api/settings" => {
                let settings: Settings = serde_json::from_value(body)?;
                library.save_settings(settings)?;
                json!({"ok":true})
            }
            "/api/cache" => library.trim_cache()?,
            "/api/video-preview" => {
                let id = body["id"].as_i64().context_or("Нет видео")?;
                library.video_source(id, true)?;
                json!({"url":format!("/video/{id}?compatible=1")})
            }
            "/api/eyes" => {
                library.eyes(
                    body["id"].as_i64().context_or("Нет фотографии")?,
                    body["faces"].as_i64().unwrap_or(0),
                    body["score"].as_f64().unwrap_or(0.0),
                )?;
                json!({"ok":true})
            }
            _ => bail!("Не найдено"),
        }
    } else {
        bail!("Метод не поддерживается")
    };
    Ok((
        serde_json::to_vec(&value)?,
        "application/json; charset=utf-8".into(),
    ))
}
