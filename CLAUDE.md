# CLAUDE.md

Guidance for AI assistants (and humans) working in this repo. Keep it short and current — update it when the build/release flow changes.

## What this is

**Claude Usage** — a Windows desktop widget that shows your live claude.ai quota (Session/Weekly plus per-model weekly limits, rendered dynamically from the API's `limits` array). Tauri 2 + Rust backend + a tiny vanilla-JS frontend, distributed as a signed MSI with an in-app auto-updater. Windows-only (uses system WebView2). End-user docs live in [README.md](README.md); this file is about working on the code.

## Layout

- **`app/`** — the Tauri app; all development happens here.
  - `app/src/` — frontend: `index.html`, `style.css`, `main.js`. **Plain vanilla JS, no bundler/build step** — `frontendDist` points at this raw folder, so edits show up on reload. The frontend talks to the backend via `window.__TAURI__` globals (`withGlobalTauri: true`).
  - `app/src-tauri/src/lib.rs` — Rust backend: the 60-second quota poll loop, the hidden claude.ai webview (cookie host + sign-in surface), the system tray, window creation (incl. `min_inner_size`), and the `#[tauri::command]`s the frontend invokes.
  - `app/src-tauri/tauri.conf.json` — Tauri config, including the updater endpoint + **public** signing key.
  - `app/src-tauri/capabilities/default.json` — the permission allowlist for the main window.
- **`index.html` (repo root) + `assets/`** — the marketing/landing page. **Not** the app UI — don't confuse it with `app/src/index.html`.
- **`docs/`** — translated READMEs (ko, zh-CN).

## Dev

```bash
cd app
npm install
npm run tauri dev      # run locally
npm run tauri build    # release MSI -> src-tauri/target/release/bundle/msi/
```

Requires Rust 1.95+, Node 20+, and Visual Studio Build Tools 2022 with the **Desktop development with C++** workload. There is no frontend build — edit `app/src/*` directly.

## Releasing

The release is a manual local build+sign+publish (no CI). Signing config lives in `app/.env` (gitignored; copy from `app/.env.example`).

1. **Bump the version in these 5 files (keep them in sync):**
   - `app/package.json`
   - `app/package-lock.json` — or just run `npm install` in `app/` after bumping `package.json`
   - `app/src-tauri/tauri.conf.json`
   - `app/src-tauri/Cargo.toml`
   - `app/src-tauri/Cargo.lock` — the `[[package]] name = "claude-usage-app"` entry

   > The settings-panel version label in `app/src/index.html` fills itself at runtime via `getVersion()` — **do not** hardcode/bump it.

2. **Build, sign, and generate the manifest:**
   ```powershell
   cd app
   .\release.ps1 -Notes "What changed in this release"
   ```
   This builds + signs the MSI, copies artifacts to `app/installers/`, and writes `app/installers/latest.json` (UTF-8 **without BOM**, signature read straight from the `.sig`). `release.ps1` expands `$HOME` in the `.env` key path, so either `$HOME\.tauri\...` or an absolute path works.

3. **Commit + tag + push:**
   ```bash
   git commit -am "vX.Y.Z: ..." && git tag -a vX.Y.Z -m "vX.Y.Z" && git push origin main --tags
   ```

4. **Publish the GitHub release with all three assets:**
   ```powershell
   gh release create vX.Y.Z --title "..." --notes "..." `
     installers\claude-usage_X.Y.Z_x64.msi `
     installers\claude-usage_X.Y.Z_x64.msi.sig `
     installers\latest.json
   ```

5. **Verify auto-update will work** — the updater endpoint is `releases/latest/download/latest.json`, so `latest.json` must ride on the newest non-draft release:
   ```powershell
   $m = Invoke-RestMethod releases/latest/download/latest.json   # (full GitHub URL)
   # $m.version should be the new version, and its signature must equal the local .sig
   ```

### Signing / updater notes

- The **private** key is at `$HOME\.tauri\claude-usage-app.key`; its password is in `app/.env`. The matching **public** key is committed in `tauri.conf.json`. **Rotating the signing key breaks auto-update for every existing user** (they must reinstall once) — avoid it.
- `latest.json` must be UTF-8 **without a BOM** (the updater's `serde_json` parse rejects a leading BOM). Let `release.ps1` generate it; never hand-edit the base64 signature.

## Conventions

- Windows + PowerShell for the release flow; the app itself is Windows-only.
- Keep changes surgical and the diff small; this is a small, focused widget.
