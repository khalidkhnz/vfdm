# vfdm

Very Fast Download Manager. Segmented parallel HTTP downloads with work-stealing re-split, resume, and a queue — for macOS and Windows — plus a Chrome extension that hands browser downloads to the app.

## Layout

- `crates/vfdm-engine` — Rust download engine (no UI deps). `vfdm-cli` test binary.
- `apps/desktop` — Tauri 2 + React desktop app.
- `apps/extension` — Chrome MV3 extension.
- `packages/protocol` — shared TS types for extension ↔ app.

## Status

Work in progress.
