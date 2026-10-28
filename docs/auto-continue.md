# Custom auto-continue build

The CLI and TUI display the upstream package version followed by
`+auto_continue`. Upstream update checks remain enabled (subject to the user's
existing startup-update preference); the update notice provides no installer
command or update action. Rebuild this fork to adopt an upstream release.

After a terminal model capability error, the TUI samples a full-jitter delay
starting with [0, 0.5) seconds. A recurrence within 64 seconds of completing
the wait doubles the sampling interval. A recurrence after an actual wait
longer than 64 seconds stops retries. After more than 64 seconds without a
recurrence, the interval resets to [0, 0.5).

Usage-limit recovery prefers the exhausted 300-minute window's reset timestamp
from account usage. It can also parse the server's local-time `try again at`
message when no snapshot timestamp is available. Unparseable reset times do
not start a timer.

During a wait, the empty composer shows an English countdown. A key press or
paste cancels the wait, restores the normal placeholder, and processes the
input normally. Existing drafts prevent a timer from starting. Switching
threads clears pending recovery. Queued messages do not bypass the wait.

When the timer expires, the TUI submits `Continue`, or dispatches `/goal resume`
if a goal was active during the interrupted turn. No configuration changes or
external wrapper processes are required.
