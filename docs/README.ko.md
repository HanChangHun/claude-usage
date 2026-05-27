# Claude Usage

[English](../README.md) | [한국어](README.ko.md) | [简体中文](README.zh-CN.md)

<div align="center">

<img src="../assets/favicon-512.png" width="180" alt="Claude Usage 아이콘">

[![Windows](https://img.shields.io/badge/Windows-10%2B-blue?style=for-the-badge)](https://www.microsoft.com/windows)
[![Tauri](https://img.shields.io/badge/Tauri-2-FFC131?style=for-the-badge)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-1.95%2B-orange?style=for-the-badge)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/License-MIT-purple?style=for-the-badge)](../LICENSE)
[![Release](https://img.shields.io/github/v/release/HanChangHun/claude-usage?style=for-the-badge)](https://github.com/HanChangHun/claude-usage/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/HanChangHun/claude-usage/total?style=for-the-badge)](https://github.com/HanChangHun/claude-usage/releases)

**실시간 Claude.ai 사용량 — Windows 데스크톱에 고정.**

[기능](#-기능) • [설치](#-설치-windows) • [동작 원리](#-동작-원리) • [개인정보](#-개인정보--보안) • [빌드](#-소스에서-빌드)

</div>

---

![Claude Usage 데스크톱 위젯](../assets/screenshot-desktop.png)

## ✨ 기능

### 🎯 핵심

- **📊 실시간 사용량 표시** — Claude.ai의 모든 한도를 한눈에: 세션(5시간), 주간, Sonnet 주간, Opus 주간, 추가 사용량 잔량까지.
- **⏱️ 60초 자동 새로고침** — 백그라운드 루프가 매분 사용량을 폴링하고, 각 한도 옆에 리셋 카운트다운을 표시합니다.
- **🪟 440×420 컴팩트 윈도우** — 데스크톱을 차지하지 않는 깔끔한 다크 위젯.
- **🎯 시스템 트레이** — 좌클릭으로 윈도우, 우클릭으로 메뉴. 최소화하면 작업표시줄에서 숨어요.
- **📦 가벼운 사이즈** — MSI ~5 MB, 런타임 메모리 ~50 MB.

### ⚙️ 설정 패널

오른쪽 상단 톱니바퀴 아이콘:

- **🚀 Windows 시작 시 실행** — 자동 시작 토글; 로그인 후 트레이에 조용히 자리잡습니다.
- **🔓 claude.ai 로그아웃** — 임베디드 웹뷰 세션을 비우고 다시 로그인 화면을 띄웁니다.
- **🔄 업데이트 확인** — 수동 트리거; 평소엔 시작 시 자동으로 체크합니다.

### 🛡️ 안전한 자동 업데이트

- **🔐 Ed25519 서명 검증** — 모든 업데이트는 임베디드 공개 키로 서명을 검증한 뒤에야 설치됩니다. 개인 키는 메인테이너 머신을 떠나지 않아요.
- **📥 GitHub Releases만** — 업데이터는 단 하나의 엔드포인트만 사용합니다.
- **🎯 무중단 설치** — 첫 MSI 설치 이후 새 버전은 다음 실행 시 자동 적용 — 재설치 없음.

---

## ⬇️ 설치 (Windows)

1. [Releases](https://github.com/HanChangHun/claude-usage/releases/latest)에서 최신 **MSI** 다운로드.
2. 더블클릭 → **추가 정보 → 실행**을 누르세요 (코드 사이닝 안 된 바이너리라 Windows SmartScreen이 경고합니다).
3. 끝. 트레이에 위젯이 뜨고, 처음 한 번 claude.ai에 로그인하면 됩니다.

> 이 한 번의 설치 이후, 모든 새 릴리스는 인앱 업데이터로 자동 적용됩니다.

---

## 🔧 동작 원리

Claude.ai 내부에는 자체 사이드바 위젯이 쓰는 사용량 엔드포인트가 있습니다:

```
GET https://claude.ai/api/organizations/<org_id>/usage
```

이 엔드포인트는 `claude.ai`에 로그인된 브라우저 탭에서만 호출 가능합니다 (same-origin + 세션 쿠키 필요). 데스크톱 앱은 claude.ai를 가리키는 **숨겨진 WebView2 윈도우**를 임베드합니다 — 처음 로그인도 여기서 일어나죠. 앱 안의 Rust 루프가:

1. 60초마다 임베디드 웹뷰에서 쿠키를 읽고 (`Webview::cookies_for_url`),
2. `lastActiveOrg` 쿠키 값을 추출하고,
3. 모든 세션 쿠키를 붙여 `reqwest`로 `/usage` 엔드포인트를 호출하고,
4. JSON 응답을 Tauri 이벤트로 발행하고,
5. 메인 윈도우가 구독해서 위젯을 다시 렌더링합니다.

쿠키가 만료되면 (연속 2회 실패 시) 임베디드 웹뷰가 표시돼서 다시 로그인할 수 있습니다.

**스택**: Tauri 2 + Rust + WebView2 (시스템) + 가벼운 vanilla-JS 프론트엔드.

---

## 🔒 개인정보 & 보안

- 🏠 **로컬에만** — claude.ai 세션 쿠키는 임베디드 웹뷰 안에만 머뭅니다 (claude.ai의 일반 브라우저 탭과 동일한 신뢰 경계).
- 🎯 **단일 엔드포인트** — 앱이 보내는 네트워크 요청은 claude.ai가 스스로에게 호출하는 것과 동일한 `/api/organizations/<org>/usage` 하나뿐입니다.
- 🔐 **서명된 업데이트** — 자동 업데이터는 GitHub Releases와만 통신하고, 바이너리 적용 전에 서명을 검증합니다.
- 🚫 **텔레메트리 없음** — 분석, 서드파티 서비스, 추적 일체 없음.
- 📖 **오픈 소스** — 모든 코드 공개; 자유롭게 감사 가능합니다.

---

## 🛠 소스에서 빌드

```bash
git clone https://github.com/HanChangHun/claude-usage
cd claude-usage/app
npm install
npm run tauri dev          # 개발 모드
npm run tauri build        # 릴리스 MSI (src-tauri/target/release/bundle/msi/)
```

Rust 1.95+, Node 20+, Visual Studio Build Tools 2022의 **C++를 사용한 데스크톱 개발** 워크로드가 필요합니다.

### 서명된 릴리스 만들기

```powershell
# 최초 1회 설정
cp app/.env.example app/.env   # app/.env를 열어 키 경로와 비밀번호 입력

# 릴리스 때마다
cd app
.\release.ps1
```

`release.ps1`은 `app/.env`(gitignored)를 읽어 `tauri build`를 실행하고, 서명된 MSI + `.msi.sig`를 `app/installers/`에 복사합니다. 두 파일과 직접 작성한 `latest.json`(형식은 `app/installers/latest.json` 참조)을 해당 버전 태그의 GitHub 릴리스에 업로드하세요.

---

## 📝 라이선스

MIT © 2026 Han Changhun
