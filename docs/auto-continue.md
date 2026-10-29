# Custom auto-continue build

The CLI and TUI display the upstream package version followed by
`+auto_continue`. Upstream update checks remain enabled (subject to the user's
existing startup-update preference); the update notice provides no installer
command or update action. Rebuild this fork to adopt an upstream release.

After a terminal model capability error, the app-server samples a full-jitter delay
starting with [0, 0.5) seconds. A recurrence within 64 seconds of completing
the wait doubles the sampling interval. A recurrence after an actual wait
longer than 64 seconds stops retries. After more than 64 seconds without a
recurrence, the interval resets to [0, 0.5).

Usage-limit recovery prefers the exhausted 300-minute window's reset timestamp
from account usage. It can also parse the server's local-time `try again at`
message when no snapshot timestamp is available. Unparseable reset times do
not start a timer.

During a wait, the empty composer shows an English countdown. A normal key press or
paste cancels the wait, restores the normal placeholder, and processes the
input normally. Existing drafts cause the TUI to request cancellation. Switching
threads only clears the local display. Ctrl+B exits the TUI without cancelling
recovery. Reconnecting receives the original server deadline. TUI queued
messages do not bypass the wait.

When the timer expires, the app-server submits an internal continuation containing `Continue`, or applies the goal-resume operation equivalent to `/goal resume`
if a goal was active during the interrupted turn. No configuration changes or
external wrapper processes are required.

The app-server listener owns the timer even with zero subscribed clients and
keeps waiting threads loaded. The server must run this custom build. Timers
are held in memory and do not survive server shutdown. The TUI never owns
a second timer that can submit work.
