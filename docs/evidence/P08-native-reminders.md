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
`--background` (no work continues after the window closes), suspend and resume, the custom
time dialog through the GUI, DST behaviour through the GUI (the policy has unit tests), and
editable morning/evening/timezone settings.
