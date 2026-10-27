#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
use anyhow::{Context, Result};
use cleaner_ultra::{library::Library, server};
use fs2::FileExt;
use std::{fs::OpenOptions, path::PathBuf};
fn main() {
    if let Err(error) = run() {
        eprintln!("Cleaner Ultra: {error:#}");
        if !std::env::args().any(|arg| arg == "--scan" || arg == "--browser") {
            let _ = rfd::MessageDialog::new()
                .set_title("Cleaner Ultra")
                .set_description(format!("Не удалось открыть приложение: {error:#}"))
                .set_level(rfd::MessageLevel::Error)
                .show();
        }
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let value = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let data = value("--data").map(PathBuf::from).unwrap_or_else(|| {
        cleaner_ultra::compat::default_data(&PathBuf::from(
            std::env::var_os("LOCALAPPDATA").unwrap_or_else(|| ".cleaner_ultra".into()),
        ))
    });
    std::fs::create_dir_all(&data)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(data.join("instance.lock"))?;
    if args.iter().any(|arg| arg == "--elevated-wait") {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            match lock.try_lock_exclusive() {
                Ok(()) => break,
                Err(error) => {
                    if std::time::Instant::now() >= deadline {
                        return Err(error).context("Предыдущее окно ещё использует библиотеку");
                    };
                    std::thread::sleep(std::time::Duration::from_millis(150));
                }
            }
        }
    } else {
        lock.try_lock_exclusive()
            .context("Эта библиотека уже открыта в другом экземпляре Cleaner Ultra")?;
    }
    let library = Library::open(&data)?;
    if let Some(root) = value("--scan") {
        library.start(&root, false)?;
        while library.progress.lock().running {
            std::thread::sleep(std::time::Duration::from_millis(100))
        }
        let progress = library.progress.lock().clone();
        println!("{}", serde_json::to_string(&progress)?);
        library.close();
        return Ok(());
    }
    let url = server::serve(
        library.clone(),
        value("--port").and_then(|s| s.parse().ok()).unwrap_or(0),
    )?;
    println!("Cleaner Ultra Rust: {url}");
    if args.iter().any(|s| s == "--browser") {
        loop {
            std::thread::park()
        }
    }
    #[cfg(all(windows, feature = "desktop"))]
    {
        use tao::{
            dpi::{LogicalSize, PhysicalPosition},
            event::{Event, WindowEvent},
            event_loop::{ControlFlow, EventLoopBuilder},
            window::{Icon, ResizeDirection, WindowBuilder},
        };
        #[derive(Debug)]
        enum WindowCommand {
            Close,
            Minimize,
            Maximize,
            Drag,
            Elevate,
            Resize(ResizeDirection),
        }
        let event_loop = EventLoopBuilder::<WindowCommand>::with_user_event().build();
        let icon =
            image::load_from_memory(include_bytes!("../packaging/cleaner_ultra.ico"))?.into_rgba8();
        let (width, height) = icon.dimensions();
        let (screen_width, screen_height) = event_loop
            .primary_monitor()
            .map(|monitor| {
                let size = monitor.size();
                let scale = monitor.scale_factor();
                (size.width as f64 / scale, size.height as f64 / scale)
            })
            .unwrap_or((1600.0, 1040.0));
        let initial_width = value("--width")
            .and_then(|s| s.parse::<u32>().ok())
            .map(f64::from)
            .unwrap_or(1440.0)
            .clamp(320.0, (screen_width - 64.0).max(320.0));
        let initial_height = value("--height")
            .and_then(|s| s.parse::<u32>().ok())
            .map(f64::from)
            .unwrap_or(940.0)
            .clamp(320.0, (screen_height - 96.0).max(320.0));
        let initial_position = event_loop
            .primary_monitor()
            .map(|monitor| {
                let origin = monitor.position();
                let size = monitor.size();
                let scale = monitor.scale_factor();
                PhysicalPosition::new(
                    origin.x + ((size.width as f64 - initial_width * scale) / 2.0).max(0.0) as i32,
                    origin.y
                        + ((size.height as f64 - initial_height * scale) / 2.0).max(0.0) as i32,
                )
            })
            .unwrap_or(PhysicalPosition::new(32, 32));
        let window = WindowBuilder::new()
            .with_position(initial_position)
            .with_title("Cleaner Ultra")
            .with_decorations(false)
            .with_window_icon(Some(Icon::from_rgba(icon.into_raw(), width, height)?))
            .with_inner_size(LogicalSize::new(initial_width, initial_height))
            .with_min_inner_size(LogicalSize::new(
                860.0_f64.min(initial_width),
                640.0_f64.min(initial_height),
            ))
            .build(&event_loop)?;
        let origin = url.clone();
        let ipc_origin = url.clone();
        let proxy = event_loop.create_proxy();
        // Keep the embedded browser cache beside the selected application data,
        // never beside the installed EXE or inside a portable/release bundle.
        let mut web_context = wry::WebContext::new(Some(library.data.join("webview")));
        let _webview = wry::WebViewBuilder::new_with_web_context(&mut web_context)
            .with_url(&url)
            .with_initialization_script(
                "Object.defineProperty(window, '__CLEANER_ULTRA_DESKTOP__', {value: true});",
            )
            .with_ipc_handler(move |request| {
                let source = request.uri().to_string();
                if source != ipc_origin && !source.starts_with(&format!("{ipc_origin}/")) {
                    return;
                }
                let command = match request.body().as_str() {
                    "close" => WindowCommand::Close,
                    "minimize" => WindowCommand::Minimize,
                    "maximize" => WindowCommand::Maximize,
                    "drag" => WindowCommand::Drag,
                    "elevate" => WindowCommand::Elevate,
                    "resize:n" => WindowCommand::Resize(ResizeDirection::North),
                    "resize:s" => WindowCommand::Resize(ResizeDirection::South),
                    "resize:e" => WindowCommand::Resize(ResizeDirection::East),
                    "resize:w" => WindowCommand::Resize(ResizeDirection::West),
                    "resize:ne" => WindowCommand::Resize(ResizeDirection::NorthEast),
                    "resize:nw" => WindowCommand::Resize(ResizeDirection::NorthWest),
                    "resize:se" => WindowCommand::Resize(ResizeDirection::SouthEast),
                    "resize:sw" => WindowCommand::Resize(ResizeDirection::SouthWest),
                    _ => return,
                };
                let _ = proxy.send_event(command);
            })
            .with_navigation_handler(move |target| {
                target.starts_with(&format!("{origin}/")) || target == origin
            })
            .build(&window)?;
        event_loop.run(move |event, _, flow| {
            *flow = ControlFlow::Wait;
            match event {
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                }
                | Event::UserEvent(WindowCommand::Close) => {
                    library.close();
                    *flow = ControlFlow::Exit;
                }
                Event::UserEvent(WindowCommand::Minimize) => window.set_minimized(true),
                Event::UserEvent(WindowCommand::Elevate) => {
                    match cleaner_ultra::space::restart_elevated(&library.data) {
                        Ok(()) => {
                            library.close();
                            *flow = ControlFlow::Exit;
                        }
                        Err(error) => {
                            let _ = rfd::MessageDialog::new()
                                .set_title("Cleaner Ultra")
                                .set_description(error.to_string())
                                .set_level(rfd::MessageLevel::Info)
                                .show();
                        }
                    }
                }
                Event::UserEvent(WindowCommand::Maximize) => {
                    window.set_maximized(!window.is_maximized());
                }
                Event::UserEvent(WindowCommand::Drag) => {
                    let _ = window.drag_window();
                }
                Event::UserEvent(WindowCommand::Resize(direction)) => {
                    if !window.is_maximized() {
                        let _ = window.drag_resize_window(direction);
                    }
                }
                _ => {}
            }
        });
    }
    #[cfg(not(all(windows, feature = "desktop")))]
    {
        loop {
            std::thread::park()
        }
    }
}
