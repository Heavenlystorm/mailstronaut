# P08 — New-mail notifications

Status: native acceptance against a disposable Dovecot and a fake notification portal on a
private bus. No real desktop notification server has been used.

## Behavior

Until this, `notifications.enabled` only covered reminders, so new mail arrived silently unless
the window was being looked at. Now, when a sync pass brings unread inbox mail and no window is
in front, the desktop is told through the standard portal.

There is one notification per account, replaced as more mail arrives, so a burst does not
produce a stack of them. It says "2 new messages" and nothing else, unless
`notifications.showPreview` is set, when it adds the latest sender and subject. A message body
never appears. Clicking it brings the inbox forward, and coming back to the window withdraws it.

Mail already there at startup is not announced, and neither is older mail fetched on request or
anything received more than three days ago. Announcing never marks anything read.

## Checks

`experiments/p04-sync/notify.sh`, called by `credentials.sh` with
`MAILSTRONAUT_NOTIFY_GUI_BINARY`, runs once per setting against a real Dovecot in a private sway
session, switching workspace to take the window out of the front:

- Mail present at startup, and mail arriving with the window in front, announce nothing.
- With the window away, a delivery is announced, and a second replaces it under the same ID.
- By default it says only "1 new message", then "2 new messages".
- With a preview it names the sender and subject, and no message body ever appears.
- With notifications disabled, nothing is shown.
- Clicking it moves the application from another view back to the inbox.
- Returning to the window withdraws the notification, and nothing was marked read.

`store::analysis` tests cover the baseline, older mail, mail already read, other folders and
other accounts. `mail_notify` tests cover the wording and removing control characters.

## Not established

Any real notification server, and how a real desktop displays or groups these. Whether a click
really raises the window on a real desktop: on Wayland that is up to the compositor. Mail that
arrives while the application is closed is not announced at the next start. Background
operation, where there is no window at all, is not implemented yet.
