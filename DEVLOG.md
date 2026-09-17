# Development Log

## 2026-09-17 — Project initialization

### Goal

Create the Tauri 2, React, TypeScript, Rust, Git, and open-source documentation baseline.

### Decisions

- Develop V0.1 on `main` with small Conventional Commits.
- Keep quota access local through the Codex CLI App Server.
- Use original SVG artwork and do not copy community pet assets.
- Target Windows verification first while preserving macOS platform boundaries.

### Validation

- `pnpm install` completed with pnpm 11.19.0.
- `pnpm build` completed successfully with TypeScript 6 and Vite 8.
- `cargo check` completed successfully with Rust 1.98.1 and Tauri 2.11.
- A repository secret-pattern scan found no credential-like values.

### Next step

Build and verify the minimal transparent desktop-pet shell.
