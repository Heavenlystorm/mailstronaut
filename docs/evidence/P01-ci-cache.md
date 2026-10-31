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

## Hosted cache acceptance and smoke-command correction

[Run 41142706847](https://github.com/eduardz1/mailstronaut/actions/runs/41142706847)
at `f6cbf815d0c1685884bbf2be138ccacc83fc6925` passed the complete minimal-container
Nix gate: release Clippy, 132 normal tests, four private credential/OAuth fixture
invocations and Nix/CI formatting. The real package restored
`nq9xmnmgnic7ai5913kzh5rp0xq9sxjm-mailstronaut-deps-0.1.0` and compiled/checked only
the five workspace crates; the previous third-party recompilation is gone.
The unsandboxed hosted builder therefore supplies the reuse evidence that the
restricted local daemon could not. The runner started with 13 GiB free and ended
the gate with 5.0 GiB free, without unrelated cleanup. This is one passing hosted
build, not a measured peak-disk bound or a second hosted source-only rebuild.

The overall run still failed in its subsequent installed smoke step:

```
nix run --no-update-lock-file -- --version
error: getting status of "/__w/mailstronaut/mailstronaut/--version": No such file or directory
```

Nix 2.34.9 reproduced the same argument-parsing failure locally. Supplying the
explicit `.#` flake target runs the already built application successfully.
The corrected workflow checks both `--version` and the native configuration
helper against `examples/config.nix`; both exact commands and workflow actionlint
passed locally through `nix develop` on Nix 2.34.9. The complete hosted workflow, including
this correction and the newer native-reader commits, needs a fresh run.

The full hosted log is `target/gpui-ci-f6cbf81-full.log`; the isolated smoke
failure is `target/gpui-ci-f6cbf81-failure.log`. A retained excerpt is linked below.

- [Hosted gate and smoke evidence](artifacts/p01-gpui-blitz/hosted-cache-gate-excerpt.log)
