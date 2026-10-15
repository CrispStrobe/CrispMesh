# CrispMesh desktop workspace

This Tauri 2 shell serves the same TypeScript workspace that can run in a browser. It creates, imports, validates, edits, and downloads project manifest v1 JSON. It keeps a local browser storage library of valid projects; download the JSON to keep a portable copy. Images and masks are referenced by paths relative to that file and are not embedded in the JSON or inspected by this workspace.

```sh
npm ci
npm run dev       # browser workspace
npm run build     # typecheck and production web bundle
npm test          # project-contract tests
npm run tauri dev # native shell; requires Rust and platform prerequisites
```

The shell exposes only `desktop_capabilities`, which reports that reconstruction is unavailable. There is no native reconstruction worker, image loader, hardware connection, or claim that an imported `complete` stage has been verified. Later worker integration belongs to D01 in `docs/PLAN.md`.

Import uses the browser's JSON parser. It enforces the v1 fields after parsing but cannot detect duplicate JSON property names; the CLI may reject such ambiguous source text. Browser import is limited to 16 MiB. Unknown metadata fields are preserved through the editor/export flow.

Verified on macOS with Node 26 and the isolated Rust toolchain: `npm run build`, `npm test` (14 contract checks), `cargo check --locked`, and `cargo build --locked` pass. The native window was not launched or packaged; platform builds beyond macOS remain to be checked.
