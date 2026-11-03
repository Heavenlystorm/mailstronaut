# P08 — Native headless lifecycle feasibility

Status: the pinned toolkit API works in the bounded native probe. This is **not**
an implementation of background mail, `--background` or headless notifications.

The test-only `lifecycle_tests` module uses the existing GPUI Kit/GPUI 0.3.10
family, `WindowingModes::WAYLAND | HEADLESS`, `QuitMode::Explicit` and
`App::request_windowing`. It starts with no display connection or window. A failed
Wayland attachment leaves the headless loop usable, then a valid attachment opens
the actual synthetic mailbox workspace. No new application framework, process
protocol, renderer or hidden window is used.

After native close, a weak workspace reference cannot upgrade. Detachment leaves
zero GPUI windows, no graphical environment and the headless compositor name.
The `/proc/self/task` observer first sees one named HTML worker while the view is
visible, then zero after each close/detach. A GPUI timer continues to publish an
atomic heartbeat with no native window. The external driver requests reopening;
the second attachment creates another actual workspace/window. Sway reports
distinct window IDs with the same process PID. After the second close/detach,
retained application state still has its expected value and the process quits
normally. This establishes a usable toolkit lifecycle for the planned session
controller; the retained test value is not an email backend.

## Acceptance

All compilation ran on ci-py through `ci/remote.sh` and pinned `nix develop`.
Strict workspace Clippy, formatting, Nix formatting, script lint and 261 normal
workspace tests passed. One scoped `future_not_send` expectation documents that
the native scenario must retain GPUI's non-Send context on its UI executor; the
root lint policy is unchanged.

The private native run passed in `target/p08-lifecycle.PtKWQW`, with synthetic
mail only, DISPLAY unset, XWayland disabled and an activation-free private bus.
The actual application test executable was selected from Cargo's full-workspace
`--no-run --message-format=json` output; the wrapper runs only the ignored native
lifecycle test. The 90-second outer timeout bounds a stalled probe.

- [Native probe receipt](artifacts/p08-headless/native-probe.log)
- [Pinned development checks](artifacts/p08-headless/development.log)

Reproduce after the full pinned check compiles the app test fixture:

```sh
MAILSTRONAUT_LIFECYCLE_FIXTURE=<current app test executable> \
  nix develop .#gui-test --command bash experiments/p08-reminders/lifecycle.sh
```

The existing application still exits when its last window closes. Next work must
move the store/auth/sync/reminder ownership out of Workspace, preserve explicit
quit and draft durability, use noninteractive credentials in background, and pass
the real close/sync/notification/reopen tests. Caller-token foreground activation,
background portal permission, suspend/resume, full HM service activation, hardware
GPU memory, models, real providers and Flatpak remain separate acceptance.

No new release/package or installed-app acceptance is claimed for this test-only
slice; the previous [single-instance package](P08-instance.md) predates it.

## Native credential prerequisite

`Store::connect_native_background` opens an explicitly encrypted oo7 Secret
Service session and looks up an existing default collection with `with_alias`.
It never calls collection creation or foreground unlock and refuses locked or
unavailable storage. It is native-only: sandbox callers must retain an encrypted
store initialized through the foreground portal, rather than use this constructor.
No new dependency/version, cipher implementation or filesystem secret format was
added. The pinned oo7 public native collection variant supplies the existing
keyring adapter.

Pinned server formatting, strict workspace Clippy and test compilation passed.
The disposable real keyring profile `target/p02.yX4vbt` passed quiet reopening,
exact account/purpose lookup, rotation/deletion, ambiguity and locked-service
checks. Independent uncached collection queries found no new collection after
removing the default alias. GNOME restores its existing login alias on a new
session, so this is **not** runtime proof of a provider with no collection at all.
Locked storage remained locked; the stopped-service check failed closed, and the
persisted fixture contained none of the synthetic credential markers in plaintext.
[Private credential receipt](artifacts/p08-headless/native-credentials.log).

Automatic sync and IDLE now use quiet credential access. A fresh private native
keyring run verified that a missing item is not created by rotation, an existing
item is updated, and ambiguous or locked items fail closed. A separate automatic
access fixture exercised cancellation and both cached and fresh access to a locked
collection, returning promptly while it remained locked. The public code paths do
not invoke unlock; the fixture did not independently monitor prompt requests.
The local OAuth fixture also exercised native automatic rotation. The synthetic
Dovecot/Wayland IDLE walkthrough passed new mail, Seen
reconciliation and reconnect using this path.

In a sandbox, an already initialized store can supply passwords and cached OAuth
access tokens, but automatic OAuth refresh is refused before contacting the server:
the pinned file-backed item update can recreate a concurrently deleted item.
The background window-independent service, sandbox permission/initialization,
provider-specific behavior and comprehensive deletion-race proof remain open.

As a prerequisite for transferring service ownership out of the window, bounded
sync and reminder update sends now observe cancellation while waiting for a full
channel. A stalled receiver therefore cannot by itself keep those tasks alive on
shutdown. This does not yet make either service independent of the window.

The reminder scheduler now has a window-independent service and relay: its latest
snapshot is retained without a view subscriber, while notification actions are
queued for an owner to consume. A private-store test observes an updated reminder
after the subscriber is dropped. A separate private-bus fixture schedules a real
synthetic reminder, observes notification delivery and receives its Open action
through the service without a GPUI view. The application now retains the reminder
entity outside the window. In a private native Wayland lifecycle probe, the first
workspace and HTML worker were destroyed, a synthetic reminder was scheduled, and
the retained entity observed it before a second window was created. The private
reminder walkthrough also passed scheduling, rescheduling, restart, cancellation
and Seen checks. Screenshots remain in ignored test artifacts.

The application also retains a single shared store owner through a Tokio
`OnceCell`; concurrent initializations use the same worker, and a reopened window
reuses it. A synthetic store test covers concurrent initialization and the profile
lock. The private Wayland lifecycle probe injects one synthetic owner and verifies
that both native windows receive that same owner across destruction and reopening.
The application configuration-loading path has not been independently exercised
through an interrupted in-flight initialization.

The sync loop and credential-access state are now also application-owned, and its
update stream is drained without a window. In the private native lifecycle probe,
an account with no available private Secret Service reports a credential failure
while no window exists. A separate private Dovecot/Wayland run passed initial sync,
unread preservation, IDLE updates and reconnection with the window open. The former
does not demonstrate successful mail synchronization while closed.

This is not background-mode acceptance: the real application still exits on last
window close, new-mail notification/indexing reactions are window-owned, and a
notification Open action is not yet routed to reopen a window. Full desktop,
Flatpak and provider acceptance remain open.
The x86_64 pinned workspace checks and full Nix package gate passed on the
`669ebd7` base, including the private credential and OAuth fixtures. Aarch64,
hosted CI, installed Flatpak, and real-provider acceptance were not run.
The gate passed again after rebasing onto `f277d89`, reusing the package and
rerunning the changed flake checks.
