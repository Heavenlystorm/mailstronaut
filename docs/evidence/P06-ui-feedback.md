# Mailbox appearance feedback — 2026-11-02

Scope: synthetic-mail layout, action affordances and configuration visibility.
No real mail or user screenshots are copied into fixtures or shipped assets.

## Implementation

- Keep the native window title; remove redundant toolbar and sidebar titles.
- Use Kit SidebarMenuItem for Settings, with one active treatment rather than a
  selected footer around a selected button. Reserve space for Kit Badge's floating
  99+ count. Use its Dot variant only for unread message rows, in either theme.
- Use Kit Button icons with accessible names/tooltips, plus AttachmentMedia with
  a file icon alongside the existing attachment name, type and size. The app
  composes Kit's default icon assets with only eight extra catalog icons using
  Kit's icon_assets macro. A test checks all action icons are embedded.
- Add standard Settings switch/input fields for priority rules and the existing
  model directory/revision declaration. Save/discard/export and managed-source
  ownership use the same native Nix preference path. The UI explicitly says model
  inference is not implemented and does not download or execute a model.
- Move custom OAuth client IDs to Advanced settings. Google onboarding explains
  that this build lacks an application-owned registration. Real Google login
  remains open; no other application's client ID is borrowed.
- Mitigate the reproduced Blitz beta.2 collapsed-table phantom border with a
  UA-level zero-width default, preserving explicit author widths. See
  [the renderer limitation](../RENDERING.md#collapsed-table-defaults).

## Verification

On ci-py through `ci/remote.sh`, `nix develop --command bash ci/check.sh`
passed formatting, script checks, strict workspace Clippy and all 273 workspace tests after rebasing onto `af56442`.
The new borderless-table regression failed before mitigation and passes after it;
inline and stylesheet explicit-border controls, selected icon asset checks and
model preference patch/clear tests pass. After adding Settings search keywords,
strict workspace Clippy and the current debug build passed again.

`nix develop .#gui-test` ran `experiments/p06-mailbox/wayland.sh` with
`MAILSTRONAUT_APPEARANCE_GUI=1`, `MAILSTRONAUT_MAILBOX_NAVIGATION=1` and
`MAILSTRONAUT_PREFERENCES_GUI=1`. The final private profile is
`target/p06.ytPgEq` on ci-py. It reports `UI_APPEARANCE_CAPTURE_OK`, verifies no
Seen changes during navigation/settings/compose, saves and exports the staged
model directory/revision, and exercises the actual Attach click through the
private test FileChooser, checking one durable attachment. Light/dark inbox and
Settings, filtered model settings, composer and attachment captures were fetched
from that exact profile into ignored `target/remote/appearance-*.png` for inspection.
The full native onboarding declaration walkthrough also passes at
`target/p06.XBfklU`, including the moved Advanced custom Google registration.
The native HTML reader walkthrough passes at `target/p01.JZBnwD`: current-frame
pixels, explicit-open-only Seen, light/dark/HiDPI, scroll/Home/End, CID retirement,
o IP socket attempts or email-specified file opens, with independent positive
observer controls. It also checks the exact copied Unicode alternatives, using
native LF clipboard line endings. This renderer-only invocation does not prove
browser portal handoff; that optional variant is separate. Its obsolete HTML-tab
coordinate was corrected (it had activated Reply), and the no-portal variant no
longer attempts a browser handoff that would add an error banner to pixel fixtures.
No X11/XWayland runtime is used. The first capture caught absent catalog icons;
asset composition fixed them before this final pass. An earlier walkthrough
failed because its final assertion incorrectly required zero drafts after Compose;
the Seen assertion now runs before creating a draft. A stale portal helper was
also rejected; the final pass uses the current compiled test executable.
This slice does not establish full-history fetching, continuous mailbox scrolling,
model inference, live OAuth consent, package/Flatpak acceptance or accessibility.
Those remain open in PLAN.md; the existing paged query is still in use.
