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

The application has not yet adopted this API in automatic sync/watcher paths.
Those still use foreground access and must be changed before headless work is
enabled. Background refresh-token persistence, sandbox permission/credential
initialization, and a combined release/package gate remain open.
