# Repository Guidelines

## Project Structure & Module Organization
- `ui/`: Tauri frontend assets (`index.html`, `app.js`, `style.css`).
- `src/`: Rust/Tauri backend project root.
- `src/src/main.rs`: app bootstrap, command wiring, tray lifecycle.
- `src/src/watch.rs`: core watcher logic for Claude/Codex/Gemini and detection rules.
- `src/src/notify.rs`: desktop notification dispatch and filtering.
- `src/src/config.rs`: persistent settings model and config file paths.
- `dist/`: packaged deliverables (`installer/`, `portable/`).
- `target/`: build artifacts (shared for dev and release).

## Build, Test, and Development Commands
- `npm install`: install Node/Tauri CLI dependencies.
- `npm run dev`: start local Tauri app in development mode.
- `npm run build` or `npm run build:nsis`: build Windows installer to `dist/installer/Aitify-setup.exe`.
- `npm run build:portable`: build portable exe to `dist/portable/Aitify-portable.exe`.
- `npm run build:all`: build installer + portable in one run.
- `cd src && cargo check`: fast Rust compile validation.
- `cd src && cargo test -- --nocapture`: run Rust unit tests with logs.

## Coding Style & Naming Conventions
- Rust: follow `rustfmt` defaults (4-space indentation, snake_case functions, CamelCase types).
- JavaScript: keep existing style in `ui/app.js` (2-space indentation, camelCase identifiers).
- Prefer focused, minimal functions in watcher code; avoid broad side effects.
- Keep user-facing text concise and in Chinese where existing UI already uses Chinese.

## Testing Guidelines
- Primary tests are Rust unit tests in `src/src/watch.rs` (`#[cfg(test)]`).
- Name tests as `test_<behavior>` (example: `test_codex_request_user_input_function_call_id_roundtrip`).
- For detection/notification changes, add regression tests for false-positive scenarios before refactoring.
- Always run `cargo test` and `cargo check` before opening a PR.

## Commit & Pull Request Guidelines
- Use Conventional Commit style seen in history: `fix: ...`, `refactor: ...`, `perf: ...`, `chore(release): ...`.
- Keep commit scope narrow; separate behavior changes from release/version bumps.
- PRs should include:
  - what changed and why,
  - impacted watcher source(s) (`claude`/`codex`/`gemini`),
  - validation commands and results,
  - screenshots for UI changes (`ui/`) when applicable.

## Security & Configuration Tips
- Do not commit machine-specific data or session logs.
- Use env vars for tuning watcher behavior (for example `CODEX_TOKEN_GRACE_MS`, `CODEX_SEED_CATCHUP_MS`, `WATCH_CONFIRM_ALERT_ENABLED`).
- Keep local config out of source control; app settings are stored via runtime config paths.
