# vfdm

**Very Fast Download Manager.** IDM-style segmented downloads for macOS and Windows, with a Chrome extension that hands browser downloads to the app.

- Splits each file into up to 16 byte ranges and fetches them in parallel over separate HTTP/1.1 connections.
- Work stealing: when a segment finishes early, its worker takes the back half of the slowest remaining segment.
- Writes every segment straight into one preallocated file at its offset. No merge step.
- Resume after pause, app restart, or crash. Validates with `ETag`/`Last-Modified` so a changed remote file fails instead of corrupting.
- Falls back to a single stream when the server ignores `Range` or omits `Content-Length`.
- Chrome extension intercepts downloads (with the page's cookies and referrer) and sends them to the app over loopback.

## Layout

| Path | What |
|---|---|
| `crates/vfdm-engine` | Rust engine, no UI dependencies. `vfdm-cli` test driver behind the `cli` feature. |
| `apps/desktop` | Tauri 2 + React desktop app. Hosts the engine and the extension bridge. |
| `apps/extension` | Chrome MV3 extension (TypeScript, Vite). |
| `packages/protocol` | Shared TS types for the extension ↔ app wire format. |

## Build from source

Requirements: Rust stable, Node 20.19+ (Vite 8), pnpm 10, and the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```sh
pnpm install
cargo test -p vfdm-engine --features cli        # engine tests
pnpm --filter @vfdm/desktop tauri dev           # run the app
pnpm --filter @vfdm/desktop tauri build         # .dmg / .msi / .exe
pnpm --filter @vfdm/extension build             # apps/extension/dist
```

Engine only, from the command line:

```sh
cargo run -p vfdm-engine --features cli -- get https://example.com/big.iso -n 8 -o ~/Downloads
```

## Pair the extension

1. Run the app. Open **Settings → Browser extension pairing** and copy the token.
2. In Chrome open `chrome://extensions`, enable Developer mode, **Load unpacked** → `apps/extension/dist`.
3. Open the extension's options, paste the token, click **Test connection**.

The app listens on `127.0.0.1` on the first free port in 7800–7810. The extension scans that range; only requests with the token and a browser-extension origin are accepted.

## Unsigned builds

Release binaries are not code-signed yet.

- **macOS** shows "vfdm is damaged" on first launch. Clear the quarantine flag once: `xattr -d com.apple.quarantine /Applications/vfdm.app`
- **Windows** SmartScreen warns on first run. Choose *More info → Run anyway*.

## Known limits (v1)

- Downloads that need a POST body or a one-time token can't be replayed by the app. The extension falls back to the browser when the app rejects them.
- No HLS/DASH video capture, no tray icon, no Firefox build yet.
- Servers that cap concurrent connections per client will answer 429 on extra segments; the engine backs off but does not yet lower its segment count per host.

## License

MIT
