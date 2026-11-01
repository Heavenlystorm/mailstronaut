# P05 — Native account declarations and first-run setup

Scope: GPUI Kit onboarding and native Nix persistence, using synthetic accounts.
Real Google/iCloud/institutional login, successful desktop/Flatpak portals and
full Home Manager generation activation remain open.

## Implementation

A missing source opens Configuration with Create native configuration. CLI and UI
share the source writer's lock, ownership marker, 0600 sibling file, no-clobber
publication and directory sync. Creating it declares no account and starts no
connection. Compose stays disabled until an account and store exist.

The Settings Add account page uses Kit Form/Field, persistent Input/SelectState,
NumberInput and ordinary buttons. Google and iCloud reuse the existing secure
presets. Manual connections support TLS/required STARTTLS, differing logins,
password/Microsoft/custom OAuth, separate outgoing passwords, aliases and folder
overrides. Collapsible contains advanced fields; InputGroup and its standard
buttons use GPUI's file chooser for additional PEM roots. A successful real
chooser/Flatpak grant has not been tested. No wizard, embedded login page or new
configuration format was added.

The full typed schema validates every candidate through the existing limited
helper. The shared checked writer appends to raw account declarations before
defaults, preserving existing omissions. It refuses stale snapshots, duplicate
IDs and unowned/managed/read-only sources. Pending preference edits must be
saved or discarded first. Fresh UUID credential references are generated for each
new declaration, independent of account ID; no password/token is serialized.
The successful canonical reload opens the new account's encrypted controls and
resets the persistent form. Export to Nix retains account metadata and references.
Home Manager's declared account list replaces the mutable list on activation;
the page explains exporting UI additions to retain them.

Successful foreground password saves and OAuth authorizations now notify the
workspace to wake that configured account's existing sync loop. Other account
loops are not explicitly woken. A private runtime test below covers password save;
it does not establish actual-provider OAuth or headless authentication behavior.

## Development and native checks

The combined source includes the current fuzzy search, priority, large-mailbox and
on-demand attachment work through `f92c7b3`. Its pinned server workspace check
passed strict Clippy and 259 normal tests, including the final account status
wording and test assertion cleanup. Formatting, Nix formatting, workflow lint and
ShellCheck also passed. No compilation ran on the laptop; a pinned formatting/
shell lint check was used there while the CI route was unavailable.
New actual-executable tests establish locked/no-clobber CLI initialization,
account append/raw omissions, duplicate refusal and stale snapshots. Three form
model tests cover presets, distinct logins/references/advanced metadata and invalid
sender/port input. The helper response boxes its typed config without changing
its wire representation; this avoids a large enum under reduced feature graphs.

The native declaration workflow passed in private Wayland profile
`target/p06.CJHMDJ` on ci-py: missing source, create, manual advanced values,
Google/iCloud declarations, native Tab/Enter submission, correct sign-in control
routing, duplicate refusal, native export/reimport and restart. It verifies mode
0600, source unchanged before Save, four distinct credential references across
three accounts and separate SMTP auth, and unchanged per-original Seen/draft/outbox
state. Automatic sync was explicitly disabled for these synthetic provider
addresses; no real provider was contacted.

The private encrypted-keyring/Dovecot profile `target/p02.NUgDFX` separately
started automatic sync without a saved password. The database remained empty.
Only saving through the actual masked native control resumed the stopped worker;
one synthetic message arrived before the 300-second poll interval, without a
manual Sync action, and both server/local copies stayed unread. Source bytes
remained unchanged. The password/purpose-isolation and unavailable-keyring tests
then passed; persisted keyring bytes contained none of the synthetic credential
markers. DISPLAY was unset and XWayland disabled in both native profiles.

Reproduce with prebuilt fixture paths from the current pinned check:

```sh
bash ci/remote.sh nix develop --command bash ci/check.sh
bash ci/remote.sh nix develop --command cargo build --workspace --all-features --locked --offline
# Under nix develop .#gui-test, run the mailbox harness with
# MAILSTRONAUT_ONBOARDING_GUI=1 and MAILSTRONAUT_MAILBOX_FIXTURE=<current worker>.
# Run credentials.sh with MAILSTRONAUT_ONBOARDING_AUTH_GUI_BINARY=<binary>
# and MAILSTRONAUT_CREDENTIAL_FIXTURE=<current credentials test executable>.
```

## Release and installed acceptance

The coordinated x86_64 Nix gate passed at
`5e79d975c8a3d3b29eb620bdc2fa4f0101f0d501`, including the integrated work through
`f92c7b3`, onboarding and reminder editing. Its numeric receipt was 0, the service
was inactive/dead with Result=success, and the complete log ends with all checks
passed. Release Clippy, 259 normal workspace tests, the additional private
credential/OAuth/web-link checks, HM module check, format and CI-script lint passed.
The gate used stable LLVM Rust 1.99, thin LTO, one codegen unit, 16 cores and one
Nix job; aarch64 was omitted. A prior stable-Clippy failure in the mailbox test
fixture was fixed with the standard integer `is_multiple_of` method.

The installed output is
`/nix/store/3a7m90176l6ywmz2497170kic7ksxmyz-mailstronaut-0.1.0`, built from
`/nix/store/dl7kv18wd0rq69w05djw82j0wk623ww0-source`. A cached `nix build` passed.
All 173 filtered source files matched the checkout, independently verified from
their SHA256 manifest. The installed CLI passed no-clobber initialization, mode
0600, typed validation and export/reimport in an explicit private path.

The same installed binary passed native onboarding in `target/p06.8ZGfhZ`,
reminder editing/restart in `target/p06.xVXNqp`, and preferences staging, discard,
save, restart, export, HM conflict, last-valid and ownership checks in
`target/p06.qRbOlb`. The encrypted password-save/automatic-sync wake test passed
again in `target/p02.HS7SUW/onboarding-auth`, followed by keyring isolation,
locked/missing-service and encrypted-at-rest checks. All profiles are synthetic
and native Wayland with DISPLAY unset and XWayland disabled. No real provider
authorization or desktop portal approval is inferred from these fixtures.

The remote runner now bounds dead connections and shares one invocation-local
OpenSSH control socket across copying/execution, including jump-host routes.
Socket cleanup stays inside the checkout. This uses standard SSH; host keys and
authentication policies are unchanged. After intermittent SSH banner failures,
terminal-backed invocations successfully ran the checks above. The cause of the
earlier connection failures was not established. The final runner passed pinned
ShellCheck and the full Nix CI-script check.

Cargo.lock includes only one application UUID dependency reference on top of the
integrated main graph. Fresh deliberately invalid-hash staging produced
`sha256-KMptW4/L3lHYhRFATs79ORN0KKza31wpEz8NIY+6OC8=`. The corrected gate's
workspace/vendor lock comparison passed before compilation.

- [Development check excerpt](artifacts/p05-onboarding/development-excerpt.log)
- [Native declaration/restart receipt](artifacts/p05-onboarding/native-declarations.log)
- [Encrypted password wake and unread checks](artifacts/p05-onboarding/native-auth-wake.log)
- [Native staged manual form](artifacts/p05-onboarding/manual-staged.png)
- [Separate incoming/outgoing encrypted controls](artifacts/p05-onboarding/manual-saved.png)
- [Release gate and installed source receipt](artifacts/p05-onboarding/package-gate.log)
- [Installed CLI, native and encrypted-keyring workflows](artifacts/p05-onboarding/installed-workflows.log)

Full logs and private profiles remain under the project target on ci-py and in
`target/remote`. Real provider authorization, CA/browser desktop and Flatpak
portals, complete HM activation and user-session accessibility remain open.
