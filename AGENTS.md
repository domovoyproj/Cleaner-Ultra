# PhotoSort development

Use Graphify first for architecture, code relationships, and change-impact analysis.

1. Install development dependencies: `python -m pip install -r requirements-dev.txt`.
2. From the repository root, rebuild with `graphify extract . --code-only` after code changes. If the Windows launcher is broken, use `.venv/Scripts/python.exe -m graphify` with the same arguments. If root auto-detection selects the `photosort` package instead of this repository, pass the absolute repository path and `--out` explicitly.
3. Use `graphify query`, `graphify affected`, `graphify god-nodes`, and `graphify explain` before inspecting and changing connected code. Verify precise behavior in source and tests; the graph is not proof of correctness.
4. Keep `graphify-out/` excluded from Git. Never commit local libraries, user photographs, tokens, build output, or virtual environments.
5. Run `python -m unittest discover -s tests -v` for changes to scanning or file operations and `node --check photosort/web/app.js` for frontend changes.
6. File removal must remain recoverable. Never overwrite an existing file during restore. Preserve crash recovery and test it.

Current runtime is Rust. Run `cargo test --locked` for core changes as well as the legacy Python regression suite above. Validate the face worker with `node --check photosort/web/face-worker.js`. Python modules preserve 0.1 migration/regression behavior and are not the 0.2 runtime.

Windows application: `cargo run --release`. Browser development: `cargo run -- --browser --port 18765 --data .photosort/dev`. Release build: `packaging/build.ps1`. On this machine without MSVC, use `cargo +stable-x86_64-pc-windows-gnu` with `.photosort/tools/mingw/mingw64/bin` on PATH. Keep `target/` ignored. Verify migrated databases and never delete the journal or original photos during cache cleanup.

Before publishing a release, verify the actual desktop application as well as the tests. Check SVG icons in both themes, initial window bounds on a small screen, disk-map selection, direct files, folder drill-down and breadcrumbs, zoom, pan, fit, expanded view and Escape. Check a leaf folder and a large paginated result. Save screenshots and record the verified scenarios. Passing syntax and unit tests alone is insufficient for a UI release.
