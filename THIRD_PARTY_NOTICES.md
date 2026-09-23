# Third-Party Notices

Codex Pet is implemented from scratch. No third-party desktop-pet source code or artwork is copied into this repository.

## Distributed software components

The desktop application is built with the following direct components. Their resolved versions are recorded in `src-tauri/Cargo.lock` and `pnpm-lock.yaml`; each component remains subject to its own license terms.

- [Tauri](https://tauri.app/) and its official Autostart and Opener plugins — desktop runtime, window, tray, installer, autostart, and shell integration.
- [React](https://react.dev/) and [React DOM](https://react.dev/) — user interface rendering.
- [Serde](https://serde.rs/) and serde_json — Rust data serialization.
- [windows-sys](https://github.com/microsoft/windows-rs) — Windows API bindings.

Build-only tooling, including TypeScript, Vite, Vitest, Testing Library, and the Tauri CLI, is also recorded in `package.json` and `pnpm-lock.yaml`.

## Architectural references

The following projects were reviewed as architectural references only:

## QimoBar

- URL: https://github.com/Picrew/QimoBar
- License: MIT
- Usage: Reference only — transparent-window, tray, and position-restoration design.

## TrizenX/agent-pet

- URL: https://github.com/TrizenX/agent-pet
- License: MIT
- Usage: Reference only — platform isolation, macOS accessory-window, multi-monitor, and DPI design.

## YijiaDuan/desktop-pet

- URL: https://github.com/YijiaDuan/desktop-pet
- License: MIT
- Usage: Reference only — Windows hit-testing and desktop-pet interaction design.

## OpenAI Codex

- URL: https://github.com/openai/codex
- License: Apache-2.0
- Usage: Protocol reference for the local Codex App Server. No Codex source code is copied.

All visual assets created for Codex Pet are original or must have a separately verified redistribution license before inclusion.
