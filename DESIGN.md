# Widget design contract

Preserve the existing compact Windows widget and vanilla JavaScript components.
The source of truth is `app/src/style.css`: warm dark backgrounds, Inter body
text, Fraunces branding, orange usage bars, amber warning and red critical states.
Reuse `row`, `row-head`, `label`, `pct`, `reset`, `bar`, `fill`, `toggle`, and
`settings-btn`. Percentages represent used quota. Do not repeat the remaining percentage.

Codex is an optional section in the scrollable usage area, enabled in Settings.
Its requests and errors are independent of Claude, triggered by the shared backend refresh signal. Default is off;
the choice persists locally. Disabling clears displayed data and stops polling.
Show the main Codex seven-day window and the five-hour window when reported; omit Spark.
Preserve the existing Claude API-driven rows: the user's current account reports
Session (5h), Weekly (all models), and Fable weekly. Do not inject legacy Opus or
Sonnet rows. These three plus one compact Codex weekly row must fit at 440x420
with settings closed. Codex has no redundant section heading above its row.
Absent windows are unavailable,
never zero. A failure clears old rows and shows login instructions; retry uses the shared top/tray refresh.

Use existing color tokens and 1rem horizontal padding. Section labels use the
existing 0.88rem row typography, helper text 0.74rem, status text 0.78rem.
The Codex row follows the same row spacing with no separate divider, refresh button or success timestamp. Do not introduce a new theme.
The app remains 440x420, supports 300x320 and scrolls when content exceeds space.
Usage and settings share one scroll container beneath the fixed header. Opening
settings scrolls them into view; no nested scrollbars or squeezed usage pane.
Use a slim rounded scrollbar with a transparent track and no arrow buttons.
Reuse the muted text colors for its thumb, with the accent color while dragging.
Keep native scrolling behavior and the system scrollbar in high-contrast mode.
Hide the thumb when idle outside the content area. Reveal it on hover and for
one second after scrolling, keeping the scrollbar width fixed to avoid layout shifts.
Buttons and toggles remain keyboard accessible;
status changes use an aria-live region. Respect reduced motion.

Accepted existing scope: Claude rendering, fonts, update flow and authentication
remain as implemented. This feature does not redesign the application or add a
web dashboard. Validate native WebView2 behavior and small-window overflow.

Account identity appears only inside the Settings panel, before its toggles.
The default usage screen contains no email, account label, or plan label.
The shared `account-info` primitive shows provider and reported plan on the left,
email on the right, using `--text-muted`, `--text`, and 0.74rem helper typography.
Use 0.6rem between account lines and 1rem before the settings controls.
Long emails truncate with an ellipsis; the full address remains available in a
tooltip and accessible text. No new controls, dividers, or animation are needed.
Claude identity comes from the widget's claude.ai session and the organization
whose usage is queried. Codex identity comes from the same CLI app-server process
as its quota. Refresh identity with usage; clear it on sign-out, errors, or disable.
An unavailable profile must not prevent quota display or imply a guessed plan.
Emails are rendered as text, kept in memory, and never persisted or logged.

The Antigravity section was removed in v0.5.1: reading its quota required running
`agy --print /usage`, which Google's terms do not allow third-party apps to run on
consumer plans. Do not add it back through `agy`.

The Local API toggle sits after the Codex toggle and before Start with
Windows. It reuses the `toggle` and `codex-help` primitives; the helper line is
an aria-live status that shows the listening URL, or the error in `--crit`.
Default off, persisted locally. It adds no usage rows, no new controls on the
default screen, and no theme changes.

Usage history opens from a quiet three-bar icon at the bottom of Settings.
Keep the compact widget geometry unchanged. The separate history window uses
the same dark palette, with orange Claude and muted lavender Codex charts.
Use actual five-minute observations, a stepped percentage line, and optional
percentage-point change bars. Never smooth, backfill, join different account
segments, or count a quota reset as consumption. Exact local timestamps must be
available through both pointer inspection and a keyboard-accessible slider.
Shared-account attribution is not inferred. Identity remains in memory; stored
history contains only quota rows, times, provider names, and opaque segments.
