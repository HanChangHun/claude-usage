# Claude Usage

[English](README.md) | [한국어](docs/README.ko.md) | [简体中文](docs/README.zh-CN.md)

<div align="center">

<img src="assets/favicon-512.png" width="180" alt="Claude Usage icon">

[![Windows](https://img.shields.io/badge/Windows-10%2B-blue?style=flat-square)](https://www.microsoft.com/windows)
[![Tauri](https://img.shields.io/badge/Tauri-2-FFC131?style=flat-square)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-1.95%2B-orange?style=flat-square)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/License-MIT-purple?style=flat-square)](LICENSE)
[![Release](https://img.shields.io/github/v/release/HanChangHun/claude-usage?style=flat-square)](https://github.com/HanChangHun/claude-usage/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/HanChangHun/claude-usage/total?style=flat-square)](https://github.com/HanChangHun/claude-usage/releases)

**Your live Claude.ai quota — pinned to your Windows desktop.**

✨ **Session • Weekly • Sonnet weekly • Opus weekly • Extra-usage** ✨

[Features](#-features) • [Install](#-install-windows) • [How it works](#-how-it-works) • [Privacy](#-privacy--security) • [Build](#-building-from-source)

</div>

---

![Claude Usage desktop widget](assets/screenshot-desktop.png)

## ✨ Features

### 🎯 Core

- **📊 Live Quota Display** — All four Claude.ai limits at a glance: Session (5h), Weekly, Sonnet weekly, Opus weekly, plus any extra-usage balance.
- **⏱️ 60-Second Auto-Refresh** — Background loop polls quota every minute; reset countdowns shown next to each limit.
- **🪟 Compact 440×420 Window** — Clean dark widget that stays out of the way.
- **🎯 System Tray** — Left-click for window, right-click for menu. Hides from the taskbar when minimized.
- **📦 Tiny Footprint** — ~5 MB MSI, ~50 MB runtime memory.

### ⚙️ Settings Panel

Gear icon, top right:

- **🚀 Start with Windows** — Toggle autostart; the app sits silently in the tray after login.
- **🔓 Sign out of claude.ai** — Clears the embedded webview session and re-prompts for login.
- **🔄 Check for updates** — Manual trigger; otherwise checked automatically on startup.

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
2. Extracts the `lastActiveOrg` cookie value.
3. Calls the `/usage` endpoint with `reqwest`, attaching all session cookies.
4. Emits a Tauri event with the JSON response.
5. The main window subscribes and re-renders the widget.

If the cookies expire (two consecutive failures), the app surfaces the embedded webview so you can sign in again.

**Stack**: Tauri 2 + Rust + WebView2 (system) + a tiny vanilla-JS frontend.

---

## 🔒 Privacy & Security

- 🏠 **Local Only** — Your claude.ai session cookie stays inside the embedded webview (same trust boundary as a normal browser tab on claude.ai).
- 🎯 **One Endpoint** — The only network request the app makes is the same `/api/organizations/<org>/usage` call claude.ai makes for itself.
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

# Each release
cd app
.\release.ps1
```

`release.ps1` reads `app/.env` (gitignored), runs `tauri build`, and copies the signed MSI + `.msi.sig` to `app/installers/`. Upload both files plus a hand-written `latest.json` (see `app/installers/latest.json` for the format) to the matching GitHub release tag.

---

## 📝 License

MIT © 2026 Han Changhun
