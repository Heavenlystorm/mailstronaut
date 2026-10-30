# GitHub CI repair, 2026-10-30

Status: repair committed as `2767c80`; the full local release/package gate passed.
No passing GitHub run is claimed. Application search integration is separate.

## Observed failures

- [Run 41054725687](https://github.com/eduardz1/mailstronaut/actions/runs/41054725687)
  at `963c1ac` failed before compilation: the pinned Snix Git fetch returned HTTP
  504. [Excerpt](artifacts/p10-ci-repair/source-fetch-failure.log).
- [Run 41030441917](https://github.com/eduardz1/mailstronaut/actions/runs/41030441917)
  at `dd3e0e3` reached SpiderMonkey but ran out of disk while producing
  `libjs_static.a`. [Excerpt](artifacts/p10-ci-repair/disk-failure.log).

The live upstream Git request also returned 504, and its official archive request
timed out. Neither documented GitHub mirror inspected had the pinned commit.
The dependency revision, source URL and fixed-output hash were therefore retained;
an unavailable source was not replaced by unverified or changed code.

## Focused repair

The Nix release environment now uses Mozilla's supported `MOZ_DEBUG_FLAGS=-g0`.
The pinned native configure source otherwise enables full DWARF by default even
in Cargo release. Both Crane passes inherit the same setting. This changes native
debug information, preserving Rust LLVM/thin-LTO/one-codegen-unit release settings
and the separate nightly Cranelift/Wild development configuration. No fork or
source patch was needed.

`ci/nix-check.sh` first fetches the existing immutable `mailstronaut.cargoDeps`
output with at most three attempts, then invokes the unchanged complete flake
gate once. Compiler, lint and test failures are not retried. The workflow retains
the pinned minimal Nix container, exact event-commit download, read-only token,
TLS verification and installed-package smoke. Disk telemetry is read-only; no
preinstalled tools, other projects or external caches are deleted.

## Local validation

- Pinned `nix develop` Nix formatting, Actionlint and ShellCheck passed.
- The actual configure-only Nix derivation passed at
  `/nix/store/0949in7v35ws2lk18xvmsarxwpxzaizx-mailstronaut-mozjs-configure-probe`.
  Its [configuration](artifacts/p10-ci-repair/native-configure.log) records `-g0`.
  The helper now avoids inheriting the real package's Clippy compilation hook,
  so this remains configuration evidence rather than a hidden full build.
- Synthetic external-command controls exercised the real CI shell entry point:
  two fetch failures followed by success invoked `build, build, build, flake`
  and exited 0; persistent fetch failure invoked only three builds and exited 42;
  a check failure invoked `build, flake` once and returned 13. These controls do
  not claim real source availability or a remote CI pass.
- The full release gate passed for exact source `2767c80`, through the same
  CI entry point, on 16 CPUs with one Nix job. Its service was
  `mailstronaut-ci-repair-nix-gate.service`, invocation
  `976fb6f6beea4c619e1df7ad313638fc`. The fresh numeric receipt is `0`, the
  service is inactive/dead with `ExecMainStatus=0`, and the complete log ends
  with `all checks passed!`. Strict release Clippy, **134 workspace tests**,
  seven isolated renderer children and five private portal/credential/OAuth child
  fixtures passed. `aarch64-linux` was omitted.
  [Receipt](artifacts/p10-ci-repair/release-gate.status),
  [excerpt](artifacts/p10-ci-repair/release-gate-excerpt.log). Full log:
  `target/p01-package-gate-service.log`.
- A subsequent cached `nix build` produced
  `/nix/store/bn6fkr2p7f053clmyx6ah4dp3ic0z7mm-mailstronaut-0.1.0`.
  The installed CLI/configuration smoke passed with an empty environment and
  `PATH=/nonexistent`, including same-binary configuration evaluation, safe
  initialization permissions/no-overwrite, and unsupported-schema rejection.
  [Installed smoke](artifacts/p10-ci-repair/installed-smoke.log).

The completed old/new dependency archives were read without extracting or deleting
files. The [measured inventory](artifacts/p10-ci-repair/native-footprint.json) shows:

| Retained files | Previous default DWARF | `MOZ_DEBUG_FLAGS=-g0` |
| --- | --- | --- |
| `libjs_static.a` | 1,120,107,302 bytes | 66,877,326 bytes |
| SpiderMonkey native build files | 2,304,273,754 bytes | 197,805,460 bytes |
| Complete compressed dependency artifact | 2,411,838,979 bytes | 1,287,437,056 bytes |

These are retained logical sizes, not a sampled build peak, hosted-runner free-disk
measurement, runtime memory improvement or end-to-end compilation speedup. The
previous output was `/nix/store/japnq7w39cpbddibx114b8dj64lmxm7h-mailstronaut-deps-0.1.0`;
the repaired one is `/nix/store/j2k9b80y20cfcldzd9qc83v4xy90ysc3-mailstronaut-deps-0.1.0`.

The local source-fetch pass used the existing hash-verified vendor cache. Retries
handle intermittent outages but cannot make a persistently unavailable upstream
service healthy. An exact-head passing GitHub run and hosted-runner disk evidence
remain required before marking the CI requirement complete.
