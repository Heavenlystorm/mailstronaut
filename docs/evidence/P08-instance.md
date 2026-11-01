# P08 — Single-instance ownership and passive activation

Status: foreground ownership/routing implemented; **P08 remains incomplete**.
Headless work, `--background`, desktop D-Bus activation, caller-token foreground
focus and Flatpak acceptance remain open.

## Implementation

The existing zbus 5.19 connection claims `io.github.eduardz1.Mailstronaut`, with
DoNotQueue and no ownership replacement, before GPUI or the mail store starts.
The interface is registered first at `/io/github/eduardz1/Mailstronaut`.
`org.freedesktop.Application.Activate` sends a passive request through a bounded,
one-entry coalescing queue. A secondary executable sends its absolute native
source location and demo bit in platform data; a different source/demo is refused
without changing the first process. Standard desktop Activate calls may omit this
hint. The location does not resolve Home Manager's changing generation symlink.
Neither route opens a selected row, changes Seen or dispatches mail actions.
File/open-action interfaces and a D-Bus-activatable desktop entry are not installed.

Acquisition/forwarding have a ten-second bound and redacted errors. A missing bus
fails closed. A forwarded launch can exit successfully without a Wayland display
or initializing GPUI. A new primary still requires native Wayland. On quit the
queue closes; after the platform stops, ownership is released and the connection
closes. Losing the bus stops the owner, including its window-owned work. The
existing core worker profile lock independently prevents a second data owner even
on a different bus. Configuration CLI commands and internal helpers bypass GUI
ownership. Last-window-close still exits normally; no background work is added.

This promotes the existing app zbus dev-dependency to a runtime dependency; no
new version or Cargo.lock change is involved. No toolkit fork, raw native handle,
unsafe code, new RPC daemon or custom widget was introduced.

## Pinned development and runtime checks

All compilation ran on ci-py through `ci/remote.sh` and pinned `nix develop`.
Before rebase, strict workspace Clippy, formatting, Nix formatting, script lint
and 259 normal workspace tests passed. After integrating `aade61b` (schema 13),
the same checks, 261 normal tests and the private bus/native ownership workflows
passed at `2e4cb91`, with an explicit numeric receipt of 0. The already-built full workspace graph supplied the app
and worker fixture executables.

`experiments/p08-reminders/instance.sh` uses an activation-free private bus.
The actual zbus interface passed exclusive ownership, eight concurrent forwarding
connections, queue coalescing, standard Activate, conflicting/malformed profiles,
closed-receiver refusal, release/reacquisition and an unresponsive-service timeout.
Profile: `target/p08-instance.ayTKzo`.

`MAILSTRONAUT_INSTANCE_GUI=1 experiments/p06-mailbox/wayland.sh` passed in private
native profile `target/p06.Z7JzzM`, with synthetic offline mail, DISPLAY unset and
XWayland disabled. It checks the bus owner's PID and exactly one native window
after eight display-independent executable launches; source/demo conflicts and a
missing bus fail without replacing it. Replacing the source with two successive
Home Manager-style generation symlinks retains the same instance identity.
A passive row selection and every launch,
normal close/restart and bus-loss shutdown leave Seen, drafts, outbox and native
source bytes unchanged. The restarted process exits normally when its private
bus stops.

## Observed focus limitation

An initial native assertion that activation returns from workspace 2 to the
existing app on workspace 1 **failed**. The final observer records workspace 2
after the request. An explicit test-compositor switch to workspace 1 provides a
positive control for that observer and exposes the app for capture; it is not an
application workaround or focus proof.

GPUI's pinned public `Window::activate_window` requests a token from its own seat.
The inspected API accepts a caller token at display attachment, but provides no
public existing-window operation to consume one. Merely forwarding platform data
cannot fix that. PLAN.md therefore keeps real caller-token foreground focus open.
Screenshots show the existing app after the explicit test-compositor switch.

The separate native demo/HTML/text regression passed in `target/gpui-wayland.RfN8aW`.
Its harness now establishes the private bus before the missing-Wayland probe,
retains a virtual keyboard, checks a blank HTML background area in both themes,
and exercises search after the reader because changing a query retires that view.
Clipboard observation is bounded and asynchronous, matching the mailbox harness.

- [Private D-Bus receipt](artifacts/p08-instance/private-bus.log)
- [Native ownership and unchanged Seen](artifacts/p08-instance/native-gui.log)
- [Existing native window](artifacts/p08-instance/owned-window.png)

## Package acceptance

The full x86_64 Nix gate passed at
`2e4cb91714594efc4164482994c37ca05e56de18`, including strict release Clippy,
261 normal workspace tests, the private credential/OAuth/web-link checks and the
new private single-instance test. HM module, format and CI-script checks passed.
Its fresh numeric receipt is 0; systemd reported success with the service
collected/inactive/dead, and the complete log ends with all checks passed.
The gate used 16 cores, one Nix job, stable LLVM Rust 1.99, thin LTO and one
codegen unit. Cargo.lock and cargoHash are unchanged; the vendor-lock guard passed.

The installed output is
`/nix/store/sir8kwznwhvig5d9pp4w8sxl6q91wfs5-mailstronaut-0.1.0`, from
`/nix/store/8zz8b6mmmk7dg5w99h2qwvcd7vywkjfd-source`. A cached `nix build`,
`--version` and typed config check passed. All 176 filtered package source files
matched the checkout, with independent local SHA256 verification.

The installed binary passed the ownership workflow in `target/p06.omVA86`,
onboarding in `target/p06.Li8yLL`, and reminder editing/reschedule/close/restart in
`target/p06.5bja0Q`; the explicit final receipt is 0. Source symlink replacements,
passive selection/activation and both restart paths preserve the same tested Seen
and local draft/outbox invariants. Foreground focus is still denied in private
Sway and remains open. The screenshot now comes from this installed package,
after the explicit test-compositor workspace switch.

- [Release and service receipt](artifacts/p08-instance/release-gate.log)
- [Installed native workflows](artifacts/p08-instance/installed.log)
- [Rebased development checks](artifacts/p08-instance/development.log)

No hosted CI, aarch64, real desktop notification service, live provider,
full HM activation, Flatpak or accessibility claim is made. P08 remains incomplete.
