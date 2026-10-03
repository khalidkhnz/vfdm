# vfdm

**Very Fast Download Manager.** IDM-style segmented downloads and video capture for macOS and Windows, with a Chrome extension that hands browser downloads and sniffed media to the app.

- Splits each file into up to 16 byte ranges and fetches them in parallel over separate HTTP/1.1 connections.
- Work stealing: when a segment finishes early, its worker takes the back half of the slowest remaining segment.
- Writes every segment straight into one preallocated file at its offset. No merge step.
- Resume after pause, app restart, or crash. Validates with `ETag`/`Last-Modified` so a changed remote file fails instead of corrupting.
- **HLS** (`.m3u8`): master playlist variant selection, AES-128 decryption, fMP4 init maps and byte ranges, separate audio renditions, parallel segment fetch written in order, resume by segment. TS output is remuxed to `.mp4` when ffmpeg is available.
- **DASH** (`.mpd`): SegmentTemplate (`$Number$`/`$Time$`/timelines), SegmentList, SegmentBase. Best video + best audio, muxed with ffmpeg or saved as two files.
- **yt-dlp**: a "Download page with yt-dlp" button for YouTube, Vimeo and other sites that need site-specific extractors. yt-dlp runs as a child process inside the same queue.
- Chrome extension: intercepts downloads (with cookies and referrer), sniffs media URLs on the current tab (badge count + popup list), right-click "Download with vfdm".

## Layout

| Path | What |
|---|---|
| `crates/vfdm-engine` | Rust engine, no UI dependencies. File, HLS, DASH and yt-dlp runners behind one queue. `vfdm-cli` test driver behind the `cli` feature. |
| `apps/desktop` | Tauri 2 + React desktop app. Hosts the engine, the extension bridge, and the tools manager. |
| `apps/extension` | Chrome MV3 extension (TypeScript, Vite). |
| `packages/protocol` | Shared TS types for the extension ↔ app wire format. |

## Build from source

Requirements: Rust stable, Node 20.19+ (Vite 8), pnpm 10, and the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```sh
pnpm install
cargo test -p vfdm-engine --features cli        # engine tests (file, HLS, DASH, resume)
pnpm --filter @vfdm/desktop tauri dev           # run the app
pnpm --filter @vfdm/desktop tauri build         # .dmg / .msi / .exe
pnpm --filter @vfdm/extension build             # apps/extension/dist
```

Engine only, from the command line:

```sh
cargo run -p vfdm-engine --features cli -- get https://example.com/big.iso -n 8 -o ~/Downloads
cargo run -p vfdm-engine --features cli -- get https://example.com/master.m3u8 --ffmpeg "$(which ffmpeg)"
cargo run -p vfdm-engine --features cli -- get https://youtu.be/… --kind ytdlp --ytdlp "$(which yt-dlp)" --js-runtime "$(which node)"
```

## Pair the extension

1. Run the app. Open **Settings → Browser extension pairing** and copy the token.
2. In Chrome open `chrome://extensions`, enable Developer mode, **Load unpacked** → `apps/extension/dist`.
3. Open the extension's options, paste the token, click **Test connection**.

The app listens on `127.0.0.1` on the first free port in 7800–7810. The extension scans that range; only requests with the token and a browser-extension origin are accepted.

Chrome will warn that the extension can "read and change all your data on all websites" (needed to read cookies and see media responses) and "read your browsing history" (`webNavigation`, used only to reset the per-tab media list when you navigate). The extension injects nothing into pages.

## Tools (optional)

Settings → **Tools** shows what was found and offers one-click installs into the app's data folder:

| Tool | Needed for | Detected from |
|---|---|---|
| ffmpeg | remuxing HLS `.ts` → `.mp4`, muxing DASH video + audio, yt-dlp merges | override path, app data, `PATH`, Homebrew |
| yt-dlp | the "Download page with yt-dlp" button | same |
| Node.js / Deno | yt-dlp's YouTube signature solving | `PATH`, Homebrew |

Without ffmpeg, HLS keeps the `.ts` and DASH saves `name.video.mp4` + `name.audio.m4a`. On Apple Silicon the downloadable ffmpeg is x86_64 and needs Rosetta; a Homebrew ffmpeg is preferred when present.

## Unsigned builds

Release binaries are not code-signed yet.

- **macOS** shows "vfdm is damaged" on first launch. Clear the quarantine flag once: `xattr -d com.apple.quarantine /Applications/vfdm.app`
- **Windows** SmartScreen warns on first run. Choose *More info → Run anyway*.

## Known limits

- Live streams (HLS without `EXT-X-ENDLIST`, dynamic MPDs) and DRM (SAMPLE-AES, Widevine) are refused with a clear error.
- Downloads that need a POST body or a one-time token can't be replayed by the app. The extension falls back to the browser when the app rejects them.
- Sites like YouTube serve throttled, IP-locked chunks; the sniffer lists them but the yt-dlp button is the intended path.
- Servers that cap concurrent connections per client will answer 429 on extra segments; the engine backs off but does not yet lower its segment count per host.
- No tray icon, no Firefox build yet.

## License

MIT
