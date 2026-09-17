# Development Log

## 2026-09-17 — Project initialization

### Goal

Create the Tauri 2, React, TypeScript, Rust, Git, and open-source documentation baseline.

### Decisions

- Develop V0.1 on `main` with small Conventional Commits.
- Keep quota access local through the Codex CLI App Server.
- Use original SVG artwork and do not copy community pet assets.
- Target Windows 10/11 only for V0.1.

### Validation

- `pnpm install` completed with pnpm 11.19.0.
- `pnpm build` completed successfully with TypeScript 6 and Vite 8.
- `cargo check` completed successfully with Rust 1.98.1 and Tauri 2.11.
- A repository secret-pattern scan found no credential-like values.

### Next step

Build and verify the minimal transparent desktop-pet shell.

## 2026-09-17 — Desktop pet shell checkpoint

### Completed

- Added an original SVG robot pet with collapsed and expanded quota states.
- Added dynamic window resizing, a drag region, transparent frameless always-on-top window configuration, and taskbar hiding.
- Added a tray menu with show, hide, and quit actions; closing the window now hides it to the tray.
- Added Vitest and Testing Library coverage for tightest-window display, expand/collapse behavior, delayed auto-collapse, and the unavailable state.
- Enabled Tauri tray support for the Windows shell.

### Verified

- `pnpm test --run`: 3 tests passed.
- `pnpm build`: TypeScript and Vite production build passed.
- `cargo check --manifest-path src-tauri/Cargo.toml`: passed after enabling the required Tauri feature flags.

### Paused intentionally

The first `pnpm tauri dev` run was stopped at compile step 352/363 to preserve the user's remaining Codex quota. The desktop window has not yet been visually inspected, so this checkpoint must not be treated as completed UI acceptance.

### Resume from here

1. Run `pnpm tauri dev` with the MSVC developer environment and `C:\Users\杨敬辉\.cargo\bin` on the process `PATH`.
2. Visually verify transparency, 164×154 collapsed size, click-to-expand, delayed collapse, drag behavior, taskbar absence, and tray show/hide/quit.
3. Fix any visual or native-shell defects, rerun `pnpm check` and `cargo check`, then replace the WIP checkpoint with a normal feature commit if desired.
4. Continue with the persistent Codex App Server adapter and real quota read.

## 2026-09-17 — Platform scope update

V0.1 is Windows-only. macOS configuration, native activation behavior, documentation, validation, signing, and distribution are out of scope.
