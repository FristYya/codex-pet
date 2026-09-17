# Codex Pet

A lightweight cross-platform desktop pet for monitoring Codex usage limits.

Codex Pet lives in a small transparent window instead of a traditional application window. It reads quota information from the locally installed Codex CLI App Server and keeps account credentials inside Codex.

## Status

Codex Pet is in active V0.1 development. The current milestone targets Windows first while keeping the application structure compatible with macOS.

## Planned V0.1 features

- Transparent, frameless desktop pet
- Live Codex quota percentages and reset countdowns
- Expandable quota details
- Windows system tray and macOS menu-bar architecture
- Position persistence and multi-monitor safety
- Local-only operation with no telemetry or backend

## Screenshot

_A screenshot will be added after the first desktop build is visually verified._

## Requirements

- Windows 10/11 or macOS
- A locally installed Codex CLI signed in with a ChatGPT account
- Node.js 24+, pnpm 11+, Rust stable, and the platform prerequisites for Tauri 2

## Development

```powershell
pnpm install
pnpm tauri dev
```

Run the frontend build with `pnpm build`.

## Architecture

- **React UI** renders the pet and normalized quota state.
- **Quota domain** converts changing Codex protocol responses into stable application types.
- **Rust Codex adapter** owns the local App Server child process and JSON-RPC transport.
- **Platform integration** owns the window, tray, click-through, and saved position.

The UI never reads Codex protocol JSON directly.

## How quota reading works

Codex Pet starts `codex app-server --stdio`, performs the initialization handshake, and calls `account/rateLimits/read`. It does not start a model turn or send a prompt.

## Privacy

Codex Pet is Local First.

- No backend and no telemetry
- No OpenAI password, Cookie, or access token input
- No reading of `auth.json`, conversations, prompts, or project source code
- No upload of quota data or personal data to a Codex Pet server

## Roadmap

### V0.1

- Codex quota monitoring
- Desktop pet UI
- Windows validation
- Basic macOS architecture
- Tray controls
- Position persistence

### Future

- Complete macOS validation
- Better animations and themes
- Automatic updates
- Optional local usage history

## License

[MIT](LICENSE)
