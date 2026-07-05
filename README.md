# Claude Usage

[English](README.md) | [한국어](docs/README.ko.md) | [简体中文](docs/README.zh-CN.md)

<div align="center">

<img src="assets/favicon-512.png" width="180" alt="Claude Usage icon">

[![Windows](https://img.shields.io/badge/Windows-10%2B-blue?style=for-the-badge)](https://www.microsoft.com/windows)
[![Tauri](https://img.shields.io/badge/Tauri-2-FFC131?style=for-the-badge)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-1.95%2B-orange?style=for-the-badge)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/License-MIT-purple?style=for-the-badge)](LICENSE)
[![Release](https://img.shields.io/github/v/release/HanChangHun/claude-usage?style=for-the-badge)](https://github.com/HanChangHun/claude-usage/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/HanChangHun/claude-usage/total?style=for-the-badge)](https://github.com/HanChangHun/claude-usage/releases)

**Your live Claude.ai quota — pinned to your Windows desktop.**

If this saves you a few quota checks, a GitHub star helps other Claude users find it.

[Features](#-features) • [Install](#-install-windows) • [How it works](#-how-it-works) • [Privacy](#-privacy--security) • [Build](#-building-from-source)

</div>

---

![Claude Usage desktop widget](assets/screenshot-desktop.png)

## ✨ Features

### 🎯 Core

- **📊 Live Quota Display** — Every limit claude.ai reports, at a glance: Session (5h), all-models weekly, and per-model weekly limits (Opus, Sonnet, Fable, …) — new models appear automatically. Extra-usage balance included.
- **⏱️ 60-Second Auto-Refresh** — Background loop polls quota every minute; reset countdowns shown next to each limit.
- **🪟 Compact 440×420 Window** — Clean dark widget that stays out of the way.
- **🎯 System Tray** — Left-click for window, right-click for menu. Closing the window hides it to the tray instead of quitting.
- **📦 Tiny Footprint** — ~5 MB MSI, ~50 MB runtime memory.

### ⚙️ Settings Panel

Gear icon, top right:

- **🚀 Start with Windows** — Toggle autostart; the app sits silently in the tray after login.
- **🔓 Sign out of claude.ai** — Clears the embedded webview session and re-prompts for login.
- **🔄 Check for updates** — Manual trigger; otherwise checked automatically on startup.
- **☕ Support on Ko-fi** — If the widget saves you time, [a coffee](https://ko-fi.com/edgetpu) keeps it maintained.

### 🛡️ Secure Auto-Update

- **🔐 Ed25519 Signature Verification** — Every update is verified against an embedded public key before installing. Private key never leaves the maintainer's machine.
- **📥 GitHub Releases Only** — Updater talks to one endpoint and nothing else.
- **🎯 Silent Install** — After the first MSI install, future versions land on the next launch — no more re-installs.

---

## ⬇️ Install (Windows)

1. Download the latest **MSI** from [Releases](https://github.com/HanChangHun/claude-usage/releases/latest).
2. Double-click → **More info → Run anyway** (Windows SmartScreen will warn about an unknown publisher; the binary isn't code-signed).
3. Done. The widget appears in your tray; sign in to claude.ai once when prompted.

> After this single install, every future release self-applies via the in-app updater.

---

## 🔧 How it works

Claude.ai has an internal endpoint that powers its own sidebar quota widget:

```
GET https://claude.ai/api/organizations/<org_id>/usage
```

Only a logged-in browser tab on `claude.ai` can call it (same-origin + session cookie). The desktop app embeds a **hidden WebView2 window** pointed at claude.ai — this is also where you sign in once. A Rust loop in the app:

1. Reads cookies from the embedded webview every 60 seconds (`Webview::cookies_for_url`).
2. Resolves the org ID: the `lastActiveOrg` cookie → last known-good value → `GET /api/organizations` (same-origin discovery, used when the cookie is missing or stale).
3. Calls the `/usage` endpoint with `reqwest`, attaching all session cookies.
4. Emits a Tauri event with the JSON response.
5. The main window subscribes and re-renders the widget.

If the session really expires (three consecutive auth failures — transient blips and challenge pages don't count), the app surfaces the embedded webview so you can sign in again, at most once every 6 hours; the widget itself always shows a sign-in button meanwhile.

**Stack**: Tauri 2 + Rust + WebView2 (system) + a tiny vanilla-JS frontend.

---

## 🔒 Privacy & Security

- 🏠 **Local Only** — Your claude.ai session cookie stays inside the embedded webview (same trust boundary as a normal browser tab on claude.ai).
- 🎯 **Same-Origin Only** — The app only calls two claude.ai API endpoints, the same ones claude.ai uses for itself: `/api/organizations` (org discovery when the cookie is absent) and `/api/organizations/<org>/usage`.
- 🔐 **Signed Updates** — The auto-updater talks to GitHub Releases and verifies signatures before applying any binary.
- 🚫 **No Telemetry** — No analytics, no third-party services, no tracking.
- 📖 **Open Source** — Code is fully public; audit anything you'd like.

---

## 🛠 Building from source

```bash
git clone https://github.com/HanChangHun/claude-usage
cd claude-usage/app
npm install
npm run tauri dev          # dev mode
npm run tauri build        # release MSI in src-tauri/target/release/bundle/msi/
```

Requires Rust 1.95+, Node 20+, Visual Studio Build Tools 2022 with the **Desktop development with C++** workload.

### Cutting a signed release

```powershell
# One-time setup
cp app/.env.example app/.env   # then edit app/.env with your key path + password

# Each release (after bumping the version — see CLAUDE.md for the 4 files)
cd app
.\release.ps1 -Notes "What changed in this release"
```

`release.ps1` reads `app/.env` (gitignored), runs `tauri build`, copies the signed MSI + `.msi.sig` to `app/installers/`, and generates `app/installers/latest.json` automatically (signature included). Upload all three — MSI, `.msi.sig`, and `latest.json` — to the matching GitHub release tag. See [CLAUDE.md](CLAUDE.md#releasing) for the full step-by-step.

---

## 📝 License

MIT © 2026 Han Changhun
