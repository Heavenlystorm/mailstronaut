# GPUI account settings and explicit inbox sync

2026-10-31. This is a bounded current-stack account workflow, not complete
P01/P02/P04/P05 or daily-client acceptance. Only synthetic mail, a private D-Bus
and encrypted keyring, and a disposable loopback TLS IMAP server were used.
No user account, dotfile activation, host keyring or real SMTP delivery was touched.

## Implementation

The ordinary GPUI Kit Sidebar exposes the unified inbox and Settings. Settings
uses `Settings`/`SettingPage`/`SettingGroup`/`SettingItem`, standard fields,
`DescriptionList` metadata and a `Clipboard` path-copy control. Static account
configuration remains in native Nix; this slice never rewrites that source.
Masked `Input` and standard buttons provide encrypted password save, browser
sign-in, explicit newest/older sync and cancellation. Full editable onboarding,
mutable Home Manager settings and export remain separate PLAN tasks.

One shared foreground-created oo7 backend and authentication session supports the
configured accounts. Password writes finish their bounded encrypted transaction
once issued. OAuth uses the existing public-client backend and ASHPD external
browser portal; no provider form or authorization URL is rendered/logged. The
new portal call is wired but this slice does not claim its desktop/provider flow.

Explicit sync reuses the existing TLS/IMAP/discovery/flag-intent/page APIs and a
shared two-connection semaphore. It downloads one bounded 128-UID inbox window,
using PEEK, the configured recent-age/body limits and the existing account-local
canonical cache. Older paging uses the returned scoped cursor. Status reports
partial coverage; this is not automatic synchronization or a complete inbox scan.
Completed or partially failed sync refreshes the conversation projection off the
UI thread. No per-account runtime, MIME engine or credential fallback was added.

Native source reload reuses the existing last-valid runtime. A successful adoption
cancels the old account lifetime explicitly, drops obsolete panes/subscriptions,
clears selected content and fences callbacks by configuration/query generation.
Invalid/missing reload retains the last valid state. Dropping the workspace
cancels account/query work. Switching views restores root focus; Ctrl+1 returns to
the inbox, avoiding GPUI's Ctrl+Shift+I debug-inspector binding.
Ctrl+K from Settings returns to the inbox and focuses its visible search field,
rather than leaving focus in a hidden entity; this is covered by the native
clipboard/query walkthrough without authorizing Seen.

The [composer reload fixture](P02-composer-reload.md) covers account changes
during submission, outbox retry and local draft recovery.

Only an explicit successful original-content open can authorize Seen. List, sync,
search, configuration reload, settings visibility and focus do not do so. Missing
readable content does not consume the authorization as a successful read. The
existing store action targets that canonical original, not every thread member.
Arbitrary cached HTML remains disabled pending P01; reading uses the standard
read-only native text component and existing bounded MIME text projection.

## Development and private native checks

`nix develop --command bash ci/check.sh`, the all-feature workspace development
build and compilation of the private test executables passed. The normal suite
has 136 passing tests; the account-action test remains ignored unless explicitly
run with the private bus/profile. It checks cancellation before credential access,
invalid-action rejection, missing older cursor and cancellation during a real
blocked foreground mutex wait, without opening the keyring.

The replacement `experiments/p02-accounts/gui.sh` runs from the existing
`credentials.sh` harness in `nix develop .#gui-test`, using prebuilt binaries so
GUI-only native inputs do not invalidate Cargo fingerprints. Its successful
development run is `target/p02.QH69DJ`; the full log is
`target/gpui-accounts-private-acceptance.log`. It verifies:

- Native Sway/Wayland, `DISPLAY` unset, XWayland disabled and private XDG paths.
- Managed native source, real Settings/account navigation, masked entry and a
  clipboard sentinel proving masked Ctrl+A/C does not expose the password.
- Save through the UI and independent `secret-tool` readback of the exact scoped
  synthetic credential, with the native configuration byte-identical afterward.
- Explicit validated TLS login, LIST and PEEK sync. The server independently
  asserts PEEK and no STORE/EXPUNGE. Two memberships share one canonical original.
- Original observed flags and no pending actions after sync, navigation and an
  invalid reload. The retained account remains usable despite the invalid file.
- Explicit click, actual body load and exact native clipboard text, followed by
  two durable Seen intents for the original's two memberships.
- App-process restart with the server stopped, exact offline body recovery and
  declared dark appearance; removing the account retains the cached original.
- No password marker in application logs or encrypted keyring bytes; the existing
  independent credential lifecycle and unavailable-backend tests also pass.

The GUI-test shell's small wlrctl patch only retains a virtual pointer when
`WLRCTL_TEST_KEEP_POINTER` is explicitly set. This gives the private compositor a
stable pointer capability; Sway IPC sends real pointer events. The harness also
retains a virtual keyboard and makes a live keyed device current before each app
start. A destroyed one-shot keyboard otherwise left the synthetic seat without
a keymap: GPUI 0.3.10 panicked on Modifiers with no Keymap. The passing reconnect
trace observes Keymap before Modifiers. No toolkit/application patch or weakened
Seen visibility check was used. This does not establish arbitrary compositor,
hardware-keyboard or real user-session acceptance.

## Package and remaining acceptance

The final coordinated service `mailstronaut-account-settings-search-gate`, invocation
`996eb30b768e408f9f505de8d3018bcf`, passed `ci/nix-check.sh` with a fresh numeric
receipt 0, a full `all checks passed` log and an inactive/collected service. The
journal also records its exit 0. Stable LLVM release Clippy, all 136 normal tests
and five private credential/OAuth/account-action invocations passed. The gate used
16 build CPUs and one Nix job on x86_64-linux; aarch64-linux was omitted. Its real
package reused the dependency artifact and rebuilt only the five workspace crates.
No timing or speedup claim is made.

`nix build --no-update-lock-file --no-link --print-out-paths .#` then passed and
returned `/nix/store/5saag2i5afhnk14ylafj625sb486i2ba-mailstronaut-0.1.0`.
All files in its filtered application source were compared byte-for-byte with
the checkout; [source/gate record](artifacts/p02-gpui-accounts/source-and-gate.json).
Earlier preliminary gates also passed, but preceded the final readable-content
or search-focus guards. They are not substituted for this final-source result.

Cargo.lock gained the app's direct reference to the already-selected ASHPD
version. A forced fresh standard vendor-staging check rejected the old fixed
output hash and measured `sha256-cH8i/Y4P8Ms72wArUgp5d/+XY6vGoZ4GSUUPFSzpNRs=`.
The corrected staging/vendor build passed; workspace and vendored raw lock hashes
both equal `sha256-KPqKyCc5NtefTi8ozmczUAUhQ71sXlQB+8XMoBE3gPQ=`. The CI guard
also checked this before compilation. No new upstream crate source was selected.

The installed package passed the complete account walkthrough above in
`target/p02.IGVFR5`, including fresh clipboard sentinels/exact source-path copy,
Ctrl+K query focus from Settings without Seen, and the actual UI credential
marker's absence from persisted keyring bytes.
The original long Unicode/read-only/Blitz image/theme/X11-rejection Wayland
regression also passed against this package (`target/gpui-wayland.NEpf03`).
Installed version/configuration smoke passed in an empty environment with a
project-local HOME. These are local installed checks, not hosted GUI acceptance.

- [Installed account Settings](artifacts/p02-gpui-accounts/installed-account.png)
- [Installed offline reader](artifacts/p02-gpui-accounts/installed-offline.png)
- [Search from Settings without opening mail](artifacts/p02-gpui-accounts/installed-search-from-settings.png)
- [Installed account walkthrough](artifacts/p02-gpui-accounts/installed-accounts.log)
- [Installed reader regression](artifacts/p02-gpui-accounts/installed-reader.log)
- [Installed CLI/config smoke](artifacts/p02-gpui-accounts/installed-smoke.log)
- [Release excerpt](artifacts/p02-gpui-accounts/release-excerpt.log)
- [Fresh vendor-staging diagnostic](artifacts/p02-gpui-accounts/fresh-vendor.log)

Full development/release logs remain under `target/gpui-accounts-components-check.log`
and `target/p01-package-gate-service.log`. Actual synthetic app screenshots were
shown in chat; the supplied third-party design references remain ignored/uncommitted.

Real Google/iCloud/institutional login, browser/locked-desktop/Flatpak portals,
Home Manager activation, automatic sync/IDLE/backoff, provider-specific folders,
large mailbox performance, accessibility and the full HTML privacy matrix remain
open. The mailbox list/reader still needs the P06 component/interaction work.
Unread indicators currently repaint on the next cache query/refresh/restart;
immediate thread-count repaint and independent query/body generations remain P06.
No complete milestone, release, live-account or hosted-CI acceptance is claimed.
