# GPUI/Blitz CI dependency-cache repair

2026-10-31. P01 remains open. This change concerns Nix source paths and artifact
reuse; it does not add HTML, accessibility, live-account or Flatpak acceptance.

## Hosted failure

[Run 41115764342](https://github.com/eduardz1/mailstronaut/actions/runs/41115764342)
at `601f67632f0b677cbedf56947cf0cf2b1a8d3bbb` failed during release test linking:
`ld.bfd: final link failed: No space left on device`. The runner started with
about 13 GiB free. Its final package recompiled third-party dependencies after
restoring Crane's dependency artifact. This was a build-disk failure, not an
observed test failure.

The old vendor preparation copied sources into each derivation's temporary
directory. Local sandboxed builders use `/build`; CI disables sandboxing and uses
different temporary directories. Cargo source paths therefore differed between
CI's dependency and application builds. The repair in `3b1694e` exposes a resolved
root `config.toml` in the standard immutable Nix vendor assembly, consumed by
Crane's existing hook. No writable source copy, dependency fork, host cleanup,
release-profile relaxation or fixed staging-hash change is introduced.

## Local validation and its boundary

The two completed builds in `target/p01-cache-reuse.YH4k6JNU` share exact vendor
and dependency derivations, but have different application source/derivations.
The variant changes the installed `--version` result with a unique marker; its
manifests and lockfile are unchanged. Both actual application builds compiled
only the five workspace crates after restoration. Both passed strict release
Clippy, 132 normal tests and four private credential/OAuth fixture invocations.
Their installed CLI/configuration checks passed, including no-overwrite behavior
and private file/directory modes. The variant marker appeared only in the variant.
Neither runtime closure contains the dependency artifact or vendor tree.

These were **sandboxed** builds. Although the initial experiment requested
`sandbox=false`, the daemon explicitly ignored that restricted setting. The
experiment exited `1` at its final unsandboxed-path assertion, after both builds
completed; the subsequent bounded checks above inspected those completed outputs
without recompiling. Do not report that experiment as a passing unsandboxed run.
The probe now supports `CACHE_REUSE_SANDBOX=false` and rejects an ignored setting
before compilation; that rejection was tested on this daemon. The default probe
requests sandboxing and reports that scope explicitly.

The subsequent coordinated `ci/nix-check.sh` gate passed with a fresh numeric
receipt `0`, full `all checks passed` log and collected/inactive service. Invocation:
`3b846b5f7fce49bba1b313c0589d7de8`. It reused the completed package, checked Nix/CI
formatting and omitted aarch64-linux. ShellCheck passed for the revised cache probe.

- Package: `/nix/store/l4vqpxa4ynmr67iy0ihx3r7fa1yz58f7-mailstronaut-0.1.0`
- Dependencies: `/nix/store/nq9xmnmgnic7ai5913kzh5rp0xq9sxjm-mailstronaut-deps-0.1.0`
- Vendor: `/nix/store/fdicpmzwbks26jncnpfy3mbhwsfxypcq-mailstronaut-0.1.0-vendor`

Full local logs, installed smoke fixtures and receipts remain under `target/`.
Hosted acceptance of the corrected source paths and runner disk use is pending;
local reuse alone does not close that gate.
