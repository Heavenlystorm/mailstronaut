# P02/P10 — Native Nix export and mutable module foundation

Status: bounded current-stack implementation; **P02/P10 and the full client remain
incomplete**. Development and package compilation run on `ci-py` through
`ci/remote.sh`, on the handed-off `claude/mail-client` branch. No user dotfiles or
real account were activated.

## Behavior and components

The optional `homeManagerModules.default` exposes `enable`, `package`, native
`settings` and opt-in `mutableSettings`. The ordinary direct-file recipe remains.
The module emits native Nix with the standard Nix generator; Rust exports with
the already-pinned Snix serializer. No extra config format, handwritten serializer,
schema mirror, parser, evaluator, or dependency fork is introduced.

`config reconcile --declared FILE --path FILE` merges exactly declared keys before
applying schema defaults. Attribute sets merge, other values/lists replace; omitted
values survive. The complete candidate is validated in the bounded same-executable
helper before atomic replacement. `accounts = []` disables declarations without
purging mail. The deployment controls ownership independently of user preferences.

The pinned bridge's `deserialize_any` string visitor returns display quotes. The raw
path therefore evaluates the standard pure `builtins.toJSON`, deserializes its
result through the correct typed String path, and decodes transient JSON internally.
The same helper/input/output/resource/timeout/cancellation boundary is retained.
Raw Debug values are redacted; no user JSON file or host Nix process is needed.

App-owned regular files carry an initialization marker. Reconciliation refuses
unadopted expressions, read-only files and unrelated links. Only a marked module
store link with the expected filename may become mutable; it is replaced without
writing through to the store. A permanent sibling lock serializes cooperating
writers, with content and inode rechecks before replacement. Explicit unlock on
drop fixes an intermittent handoff conflict observed during parallel helper tests.
This is cooperative locking and stale-snapshot detection, not an adversarial
filesystem compare-and-swap guarantee against an uncooperative editor.

Settings offers Export to Nix through the standard Kit Dialog, persistent read-only
Textarea, Clipboard, portal Save Button and in-app Notification. It snapshots the
last valid config, with effective defaults, accounts and keyring references; it
omits ownership, credentials and runtime/mail state. Export work runs off the UI
thread; configuration revision and current view fence its result. Saving uses a
private temporary sibling and no-clobber publication, preserving existing files
and links, including the active source.

## Checked behavior

- Server-side pinned `ci/check.sh` passes format/tool lints, strict workspace
  Clippy and 178 normal tests. Four new actual-executable configuration tests cover
  first/repeated activation, defaults/omissions/lists, invalid roots, exact strings,
  escaping, cancellation/recursion, readonly/unadopted sources, lookalike links,
  separate-process writer exclusion, stale content/inode and lock-link rejection.
- The lock release correction passes ten repeated four-test runs and the full
  workspace check. Unit tests also cover native round trips, raw Debug redaction,
  private/no-clobber export publication against files and symlinks, and expanded
  configurations whose serialized export would exceed the 256 KiB source limit.
- The private Wayland export fixture uses the existing synthetic mailbox harness,
  with private HOME/XDG/bus, software Vulkan, `DISPLAY` unset and XWayland disabled.
  Both Clipboard and text selection copy the exact CLI snapshot, the native preview
  rejects typing, the copied result validates through the same binary's limited helper, and
  source bytes and per-original Seen state remain unchanged. Light/dark rendering
  and a missing FileChooser portal are exercised. The latter displays an error.
- The Nix module check evaluates the actual module with **stubbed HM plumbing** and
  runs its generated activation against the installed package in a disposable
  HOME. It covers native managed data, mutable activation/precedence/retention,
  empty account lists, invalid-declaration retention, export reimport, module-link
  adoption without changing the store file, and rejection of an unrelated link.

## Release and installed package

The corrected-source x86_64 Nix gate passed through `ci/remote.sh` and
`experiments/p01-nix/run-gate.sh`, using the server's CPUs and one full Nix job.
The fresh receipt is zero and the full log reports all checks passed. Stable LLVM
release Clippy, 178 normal tests and six private checks passed; the module check
also passed. The aarch64
system was omitted; no hosted-CI result is claimed.

`nix build --no-update-lock-file` produced
`/nix/store/wdsyks34blr1ybv5nmcm57bqx47sks26-mailstronaut-0.1.0`.
All 149 filtered files in
`/nix/store/x3lb677blgks1d1wcran3m53a9p5igb6-source` match the checkout.
Cargo.lock is unchanged; the standard vendor-lock guard passed. Moving the existing
Serde JSON reference from dev to normal dependencies introduced no new version.

The installed binary passed version/config/export/reimport smoke. Its private
Wayland export fixture passed in `target/p06.Ho84XY`, with exact Clipboard/text
copy, readonly behavior, source/Seen retention, themes and missing-portal feedback.
The installed navigation regression passed in `target/p06.XmMHcn`, including pages,
Sent ancestry, account retirement, offline restart, explicit unread/star, a reply
draft with ancestry and a durable one-hour reminder. Both profiles are on `ci-py`.

- [Package source hashes](artifacts/p02-native-nix/source-manifest.json)
- [Gate receipt](artifacts/p02-native-nix/gate-receipt.txt)
- [Release test excerpt](artifacts/p02-native-nix/release-excerpt.log)
- [Installed CLI/config smoke](artifacts/p02-native-nix/installed-smoke.log)
- [Installed mailbox regression](artifacts/p02-native-nix/installed-navigation.log)
- [Light export dialog](artifacts/p02-native-nix/light.png)
- [Dark export dialog](artifacts/p02-native-nix/dark.png)
- [Unavailable save portal](artifacts/p02-native-nix/save-unavailable.png)

The complete current-source release log is retained in
`target/remote/hm-release-build.log`; `nix log .` retrieves the immutable package log.

## Remaining acceptance

Basic editable preferences were subsequently implemented and tested in
[P02-preferences.md](P02-preferences.md). Account onboarding, complete Home Manager activation
and generation collision/backup behavior (not stubbed module plumbing), successful
desktop FileChooser save/cancel/denial, real desktop accessibility, Flatpak runtime,
and live provider authorization remain on PLAN.md. The toolbar appearance toggle
still previews a window theme; it does not persist a preference. Native export is
not evidence that these other settings are implemented.

## Local native FileChooser export

The current native app (`15cd9a8`) exercised the real Export to Nix dialog
against the private FileChooser service. Four `SaveFile` receipts covered
cancellation, refusal, successful save to a Unicode filename, and an existing
destination. The successful file had mode `0600`, matched the CLI export byte
for byte, and passed the application's evaluator. The pre-existing destination and
active declaration stayed byte-identical, and Seen did not change. The evidence
is in `target/p06.udWsOt`. This uses a synthetic portal on the private bus;
the live desktop and Flatpak Documents-portal grant remain open.
