# Claude Usage Web

A static, single-file site that mirrors your **Claude.ai quota** — Session (5h), Weekly, Sonnet/Opus Weekly, and any extra-usage balance — and keeps it refreshing every 60 seconds as long as a claude.ai tab stays open.

Live: **https://hanchanghun.github.io/claude-usage-web/**

## How it works

Claude.ai has an internal endpoint that powers its own sidebar quota widget:

```
GET https://claude.ai/api/organizations/<org_id>/usage
```

Only a logged-in browser tab on `claude.ai` can call it (same-origin + session cookie). An external static site can't reach it due to CORS. So this project uses a **console snippet bridge**:

1. You paste a short snippet into the DevTools Console on `claude.ai`.
2. On first run the snippet `window.open`s this page with the usage JSON base64-encoded in the URL hash.
3. The snippet then runs every 60 seconds, pushing fresh data to this page via `postMessage` — no new tab, no focus theft.
4. This page decodes each update, re-renders the widget, and caches the latest snapshot in `localStorage` so you see the last known values even if the claude.ai tab closes.
5. The channel is two-way: the **Refresh** button on this page posts a `tick` command back to the claude.ai tab, which fetches immediately instead of waiting for the next 60-second tick.

No backend. No extension. No API key. Your claude.ai session cookie never leaves `claude.ai`.

> **Why not a bookmarklet?** claude.ai sets a strict Content Security Policy that blocks `javascript:` bookmarks — clicking one just redirects to `about:blank#blocked`. The DevTools console sidesteps CSP because it's privileged.

## Use

1. Open [the page](https://hanchanghun.github.io/claude-usage-web/).
2. Click **Open claude.ai ↗** to launch a linked claude.ai tab. Tick **mini window** first if you'd like it to open as a compact popup window you can park on another virtual desktop, a second monitor, or minimized.
3. In that new tab/window, press <kbd>F12</kbd> (or <kbd>⌘</kbd>+<kbd>⌥</kbd>+<kbd>I</kbd> on Mac) → **Console**.
4. Back on this page, click **Copy** to copy the snippet, then paste it into the console and press <kbd>Enter</kbd>. If Chrome asks, type `allow pasting`.
5. Your quota appears here and refreshes automatically every 60 seconds as long as the claude.ai tab/window stays open.

**Refresh now:** click the **Refresh** button in the widget header to ask the claude.ai tab for an immediate update (it stays disabled until the snippet is running).

**Stop auto-refresh** at any time: run `clearInterval(__cuw.iv)` in the same console, or just close the claude.ai tab. Reloading claude.ai also stops it; re-paste to resume.

## What you see

| Field | Source |
|-------|--------|
| Session (5h) | `five_hour.utilization` + `resets_at` |
| Weekly | `seven_day.utilization` + `resets_at` |
| Sonnet weekly | `seven_day_sonnet.utilization` (Max plan only) |
| Opus weekly | `seven_day_opus.utilization` (Max plan only) |
| Extra usage | `extra_usage.used_credits / monthly_limit` (if enabled) |

Reset countdowns re-render locally every 30 s. A green dot + pulse animation marks every fresh push; the dot turns amber and the badge flips to "stale" after 10 minutes without an update.

The **Refresh** button is enabled as soon as the first push arrives and goes disabled again if the claude.ai tab closes. While a manual refresh is in flight the icon spins; it clears on the next push or after a 5-second timeout if the claude.ai tab didn't respond (e.g. after a reload without re-pasting).

### Mini window mode

The **mini window** toggle next to the *Open claude.ai* button asks the browser to open claude.ai as a **compact popup window** (~480×360) instead of a regular tab. Chrome and Edge respect this hint reliably; from there you can drag the window to another virtual desktop (Windows: right-click the taskbar entry → *Move to desktop*; macOS: move to another Space), a second monitor, or just minimize it out of the way. Auto-refresh keeps working — Chrome throttles background/hidden windows, but 60-second intervals generally survive intact.

## Privacy

- The snippet runs only when *you* paste it, in the tab *you* pasted it into.
- The only network call it makes is the same call claude.ai already makes for you — `/api/organizations/<org>/usage`, with your existing logged-in session.
- The first response travels to this page via URL hash; subsequent ones via cross-tab `postMessage` — both stay entirely in your browser.
- The latest snapshot is cached under `localStorage` key `claude-usage-web:v2`.
- Clear everything anytime: DevTools → Application → Local Storage → delete the key (and close the claude.ai tab to stop the loop).

## Limitations

- **Desktop browser only.** Mobile Safari / iOS Chrome don't have a DevTools console to paste into.
- **The claude.ai tab must stay open** for live updates. If it reloads or closes, refresh is paused; open the tab again and re-paste to resume.
- Anthropic could change the endpoint shape without notice. If the widget shows "No limits reported," the response likely shifted.

## Files

- `index.html` — the whole app (styles, script, snippet generator)
- `favicon.svg` — the quota-mirror mark
- `LICENSE` — MIT

## License

MIT © 2026 Han Changhun
