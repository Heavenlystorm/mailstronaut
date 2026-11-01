# P02/P08 — Editable native preferences

Scope: basic preferences in the GPUI Kit Settings view. Account onboarding,
complete Home Manager generation activation, background lifetime and desktop/
Flatpak accessibility/portal acceptance remain separate PLAN requirements.

## Implementation

The standard Kit dropdown, switches and number field stage appearance, automatic
synchronization/poll interval and reminder notification/privacy preferences.
Persistent segmented `TimeField`s and a searchable `Combobox` backed by the
existing Chrono TZ catalogue stage morning/evening and timezone. A standard
Toolbar keeps Save/Discard outside the filtered Settings body. DescriptionList
shows source/ownership/last reload; persistent Alerts show reload/save failures.
Both Combobox Change and Confirm are handled because this pin may reuse a filtered
row index and omit Change. The native workflow must cover the actual saved value.

The config crate shares activation and UI candidate validation/publication. UI
saves require an app-owned writable regular file and an unmanaged loaded config;
they compare the currently evaluated effective config against the loaded snapshot.
They patch raw explicit keys before defaults, validate the entire result through
the bounded helper and atomically replace the source under the existing sibling
lock. The ownership marker counts toward the source budget. No preferences database,
secondary format, custom serializer, parser or evaluator is added. Only the
already-locked Tempfile reference moves into the config crate's normal dependencies.

Successful Save uses the existing canonical reload/adoption path to apply theme,
timezone, reminders and account/sync configuration. Failure retains the last valid
runtime; stale saves require Reload. Explicit ownership checks protect managed,
read-only, user-authored and unrelated linked sources. Credential actions stay
separate from declarative preference ownership.

## Checks

The rebased server-side `nix develop --command bash ci/check.sh` passed strict
workspace Clippy and 201 normal tests after integrating the current send/signature changes. New actual-executable tests cover raw-key
omissions, changed-source/HM precedence, typed validation, concurrent writers,
read-only/link ownership and cancellation without replacement. Two source-writer
tests cover explicit lock release with a duplicated open description and the
ownership-prefix input budget. All use synthetic data/private profiles.

## Native workflow

The private native Wayland workflow passed in `target/p06.sjuAhZ` on `ci-py`,
using the debug executable with private HOME/XDG/bus, synthetic mail, software
Vulkan, DISPLAY unset and XWayland disabled. It exercises native Settings search,
segmented time editing, a filtered timezone selection, Save/Discard, source bytes
unchanged until Save, mode 0600, restart/export/reimport equivalence, HM declaration
precedence, stale-save refusal, last-valid retention after an invalid reload, and
managed/authored-source disabled editing. Per-original Seen, drafts and outbox
remain unchanged throughout. HM activation here means the actual reconcile CLI;
it is not a complete Home Manager generation activation.

Discard checks the native time field's pixel signature against its baseline.
The saved canonical export verifies the actual timezone value independently of
what the picker displays; this caught the missing Change event described above.

## Release and installed acceptance

The final x86_64 server Nix gate passed under the shared global flock: fresh
receipt zero, all checks passed, strict stable-LLVM release Clippy, 201 normal
workspace tests plus six private checks, and the installed module check. Thin
LTO and one codegen unit remain the release policy; no new compiler exception
was needed. The aarch64 system was omitted; hosted CI was not awaited.

Cargo.lock adds only the already-locked Tempfile dependency reference. A deliberate
fake-hash staging probe produced
`sha256-StiZf0KuQ5eqP0uBNvX81FWQob/4WV2Zbt4CfiWewPo=`; the corrected gate passed
the workspace/vendor lock guard and actual builds. The send/signature
rebase did not change these lock bytes.

Final package: `/nix/store/whcamjh1wg4bhcrx5sp3r643il465xr5-mailstronaut-0.1.0`.
Its filtered source is `/nix/store/z2xm4hz0v5dcfzqadrdvlmf73vfwiwag-source`.
All 154 filtered files match the server checkout and were independently checked
against the local checkout using the recorded hashes. The final README and native
configuration documentation are included in that source. Remaining evidence and
PLAN updates do not change the filtered package inputs.

The installed binary passed CLI init/check/export/reimport and the complete private
native workflow again in `/builds/mailstronaut-dev/target/p06.G7N0pw`. It verifies staging/discard,
all saved fields (including the actual timezone), mode 0600, restart/export/copy,
HM static precedence and undeclared retention, stale-save refusal, persistent
invalid-reload/last-valid behavior, managed/authored-source protection and unchanged
Seen/draft/outbox state. Use the fixture executable reported by the current passing
workspace check (`worker-5124c1d2dab8a8fe` here); a cached pre-signature fixture
correctly rejected the newer helper response and was not accepted as a GUI result.

The server's inactive project target was moved, preserving all contents, from
`/builds/mailstronaut-dev/target` to `/mnt/data/mailstronaut/dev-target`, with the
original path retained as a symlink. The move touched only this project's target.

- [Gate receipt](artifacts/p02-preferences/gate-receipt.txt)
- [Release excerpt](artifacts/p02-preferences/release-excerpt.log)
- [Fresh vendor staging probe](artifacts/p02-preferences/vendor-staging-excerpt.log)
- [Filtered source hashes](artifacts/p02-preferences/source-manifest.json)
- [Installed CLI smoke](artifacts/p02-preferences/installed-cli.log)
- [Installed native acceptance](artifacts/p02-preferences/installed-native.log)
- [Native light Settings](artifacts/p02-preferences/light.png)
- [Persisted dark theme and reminder time](artifacts/p02-preferences/dark.png)
- [Stale-save Alert](artifacts/p02-preferences/conflict.png)

Complete release/installed logs remain in `target/remote/prefs-source-final-release.log`
and `target/remote/prefs-installed.log`; `nix log .` retrieves the package build log.
Full Home Manager generation collision/backup/activation, successful file portals,
user-session accessibility, Flatpak and live-provider authorization remain open.
