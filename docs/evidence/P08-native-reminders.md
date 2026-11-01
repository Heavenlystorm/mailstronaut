# P08 — Native reminders and notification delivery

Status: bounded current-stack implementation; **P08 remains incomplete**. All mail is
synthetic. No real desktop notification server or Flatpak portal has been used.

The reader's Remind me menu offers In one hour, This evening, Tomorrow morning and Pick a
time (Kit `DatePicker` with minute precision). Presets resolve through the retained
civil-time policy in the declared timezone, else the system's, else UTC. Mail dates now
use that same zone. Reminders (Ctrl+3) lists scheduled, overdue, notified and failed
reminders, with Open, Cancel/Dismiss and Retry, so a muted or missing notification
service never hides one. The sidebar badge counts overdue reminders.

A Tokio loop owns delivery. It sleeps until the earliest due reminder, for at most ten
minutes so clock changes are noticed. The store remains the sole durable owner of each
transition. Desktop actions name a reminder and are resolved against current state, so a
stale notification cannot act on a rescheduled or removed one. Opening a reminder is an
explicit open, and it is the only way a reminder can mark mail read.

## Checks

Run on the CI server through `ci/remote.sh`.

- `experiments/p08-reminders/portal.sh` runs the retained test on a private bus with a fake
  `org.freedesktop.portal.Notification`: stable replacement across both crash windows,
  scoped actions, generic previews by default, mute, denial, explicit retry and withdrawal.
- `MAILSTRONAUT_MAILBOX_NAVIGATION=1 experiments/p06-mailbox/wayland.sh` opens a message,
  chooses In one hour through the real menu, and finds one pending durable reminder about
  an hour away in SQLite and in the Reminders list.

## Not established

Real desktop notification servers and their buttons, single-instance D-Bus activation and
`--background` (no work continues after the window closes), suspend and resume,
evening/tomorrow presets and DST behaviour through the GUI remain unverified
(the policy has unit tests). Editable morning/evening/timezone settings now have their own
[native preference acceptance](P02-preferences.md).

## Time details and rescheduling

The reminder list now wires Kit `DescriptionList` to the stored local time,
original IANA timezone and UTC instant. A Reschedule button opens the existing
Kit `DatePicker`. Each opening seeds that original's future reminder, or tomorrow's
configured morning, in the current configured timezone. It no longer carries a
different original's picker selection into the new dialog. At narrow widths, the
same component uses one column instead of three. The existing durable
schedule API still owns generation checks and publication; no second scheduler or
date editor was introduced.

The pinned server workspace check passed strict Clippy and 259 normal tests before
the final layout adjustment. Afterwards, targeted app Clippy, all 38 normal app
tests and the native build passed again, along with formatting, Nix formatting and
ShellCheck with sourced scripts. No compilation ran on the laptop.

`MAILSTRONAUT_REMINDERS_GUI=1` in `experiments/p06-mailbox/wayland.sh` passed on
ci-py in private profile `target/p06.yH4KIY`, using synthetic offline mail, DISPLAY
unset and XWayland disabled. It verifies:

- Opening and cancelling the custom dialog creates no reminder.
- Native calendar selection and segmented time entry persist the exact original,
  UTC instant and Europe/Paris timezone.
- Reschedule retains original/notification identity and increments the generation.
- A second original starts with tomorrow morning, independently of the previous
  11:42 selection.
- Both reminders survive normal last-window-close and offline restart unchanged;
  the process exits normally with background disabled.
- Cancellation, list navigation, theme/size changes and restart preserve Seen,
  the native configuration and the empty drafts/outbox.

Screenshots were inspected at 1280×840 and 960×720 and in both themes; the current
native app was shown in chat. This private bus intentionally has no notification
service. A retained withdrawal failure after rescheduling is visible above the
list; it neither loses nor cancels the reminders. The native test accounts for
that status line. Actual desktop notification presentation remains unverified.

- [Native runtime receipt](artifacts/p08-native-editor/native-gui.log)
- [Development checks](artifacts/p08-native-editor/development-excerpt.log)
- [Custom calendar and time](artifacts/p08-native-editor/calendar.png)
- [Stored time details](artifacts/p08-native-editor/details.png)
- [Narrow layout](artifacts/p08-native-editor/narrow.png)
- [Dark appearance](artifacts/p08-native-editor/dark.png)

The full x86_64 release/package gate passed at
`5e79d975c8a3d3b29eb620bdc2fa4f0101f0d501`, with strict release Clippy, 259 normal
workspace tests and the additional private credential/OAuth/web-link checks.
The same native reminder workflow then passed against the installed package in
`target/p06.xVXNqp`, including close/restart, identity/generation and Seen checks.
The package path, numeric receipt, source comparison and bounded installed results
are recorded with [onboarding package acceptance](P05-onboarding.md#release-and-installed-acceptance).
There is no hosted-CI, aarch64, live-account, real notification server, Flatpak,
full HM activation or accessibility claim. P08 remains incomplete.
