# Third-Party Notices

Codex Pet is implemented from scratch. No third-party desktop-pet source code or artwork is copied into this repository.

## Distributed software components

The desktop application is built with the following direct components. Their resolved versions are recorded in `src-tauri/Cargo.lock` and `pnpm-lock.yaml`; each component remains subject to its own license terms.

- [Tauri](https://tauri.app/) and its official Autostart and Opener plugins — desktop runtime, window, tray, installer, autostart, and shell integration.
- [React](https://react.dev/) and [React DOM](https://react.dev/) — user interface rendering.
- [Serde](https://serde.rs/) and serde_json — Rust data serialization.
- [windows-sys](https://github.com/microsoft/windows-rs) — Windows API bindings.
- [OpenAI Codex runtime](https://www.npmjs.com/package/@openai/codex) — bundled only in Windows x64 release builds so Codex Pet can run the local App Server without a separately installed CLI. The pinned input is `@openai/codex@0.156.1-win32-x64`, downloaded from the official npm registry and verified against its published SHA-512 SRI (`sha512-MJyLxbBs2zzp5kbaR/99Zwe7SmbrwUkveTcT+ayYlO48V0nYh0eU+h2lalBwvC7VJ/ya/bXnUtISJfJKhGCD/g==`). It is Apache-2.0; release packages include `resources/codex-runtime/CODEX-RUNTIME-LICENSE.txt` and retain the notices and licenses for included runtime components. Codex Pet does not use voice features, so the optional native voice resource directory is omitted from the installers.
- [ripgrep](https://github.com/BurntSushi/ripgrep) 15.2.0 (revision `e89fff89ac`) — distributed inside the pinned Codex runtime as `codex-path/rg.exe`. ripgrep is dual-licensed under MIT and Unlicense; this distribution uses MIT, with the full copyright and license text at `resources/codex-runtime/RIPGREP-LICENSE-MIT.txt`.

The Windows installers also contain this project's `LICENSE.txt` and this `THIRD_PARTY_NOTICES.md` in the application resources directory.

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
- Usage: Protocol reference and upstream App Server runtime. No Codex source code is copied into Codex Pet; the optional voice resource directory is omitted.

All visual assets created for Codex Pet are original or must have a separately verified redistribution license before inclusion.
