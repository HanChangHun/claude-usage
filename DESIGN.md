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
