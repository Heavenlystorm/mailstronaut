# P01 Nix dependency artifact reuse

Status: local x86_64 Nix dependency-cache acceptance passed. P01 remains open.
GUI, accessibility, live-account, remote CI and current-source Flatpak acceptance
are not established by this build work.

The initial Crane experiment compiled and passed its 35 release tests, but
recompiled dependencies after importing its archive. Its adapter copied writable
vendor sources without retaining timestamps. The pinned Crane archive hook
normalizes target artifact timestamps to epoch 1, so newly timestamped sources
appeared newer than the restored artifacts. The initial result and its limits
are preserved in `artifacts/p01-cache/initial-experiment.md`.

The timestamp correction was first tested against the same existing 2.2 GiB
artifact as a diagnostic, with no production hardcoded artifact reference. Only
the four workspace crates compiled/checked after import; no third-party crate
was rebuilt. Release build, strict release Clippy and test compilation took
6m07s, 52.49s and 5m13s respectively. All 35 tests and seven renderer children
passed, with a total service receipt of 775 seconds. The diagnostic output was
`/nix/store/rjgl1rybwy7sy05rgi52hhigwnp3cnz8-mailstronaut-0.1.0`; its runtime
closure did not contain the dependency artifact. The complete diagnostic log
and receipt are committed under `artifacts/p01-cache/`. Phase times are local
observations with uncontrolled system load, not a benchmark ratio.

The normal implementation derives artifacts with pinned Crane 0.24.0
(`692f7e9ef2ece8125b466f66f2af532b3edaed0d`). Both dependency and final package
share native inputs, stable LLVM release settings, target/features, offline
vendoring and timestamp-preserving preparation. The existing five nested mozjs
lock substitutions and native assembler workaround remain. The nightly-only
`.cargo` configuration stays excluded; root thin LTO and one codegen unit stay
unchanged. Strict release Clippy and actual workspace tests remain mandatory.

The vendor fetch now receives a lock-only source. This also removes the initial
experiment's dependency derivation changes when application source changes.
Before compilation, independent evaluations with an actual Rust-comment change
showed identical vendor and dependency derivations/output paths, and different
application source/package paths. The actual source-only package build below
then established reuse; the identity check alone would not have been enough.

`experiments/p01-nix/cache-reuse.sh` provides that acceptance. It keeps its
fixture, derivation identities, complete package logs, elapsed times, installed
smokes and runtime-closure checks under a fresh project-local target directory.
It holds the shared build lock, changes source contents without changing Cargo
manifests/lock, rejects third-party recompilation, and verifies both installed
outputs. Its exit receipt distinguishes completion from interruption. The
retained artifact must not become an installed runtime dependency.

## Normal package gate

The normal gate on base HEAD `2ceb35d6f7e591306ce28db5bfd473a414880192`
with the working-tree Crane changes passed in transient service invocation
`c24dcf7c09414e98b027f20da31ea51d`. Its fresh receipt was `0`, the service
was inactive/collected, and the log ended with `all checks passed!`. The command
was `nix flake check --no-update-lock-file --print-build-logs --cores 16
--max-jobs 1`. The committed `artifacts/p01-cache/normal-gate.log` records the
whole run, including the first dependency artifact build.

The first dependency pass ran release check (8m48s), build (9m28s) and dummy test
compilation (5.94s); dummy compilation is not application-test acceptance.
The real package then imported the archive and compiled/checked only the four
workspace crates. Its release build took 6m20s, strict workspace all-target and
all-feature release Clippy 52.31s, and release test compilation 5m27s.
All 35 workspace tests and seven isolated renderer child invocations passed.
Local Nix formatting and CI lint checks passed too. The gate omitted
`aarch64-linux`; it did not run remote CI.

The dependency output is
`/nix/store/qfbhx27m06xphb9x4ajrg3v776lr1mni-mailstronaut-deps-0.1.0`,
with a compressed archive of 2,373,256,274 bytes (about 2.2 GiB). The normal
package derivation is
`/nix/store/xfjrafasr8jwgb6xaw49qcsgxcq0fm3s-mailstronaut-0.1.0.drv`,
output `/nix/store/rqnjx2fqp7ydh8a1jmssivi5clxfzd2b-mailstronaut-0.1.0`.
A subsequent normal `nix build --no-update-lock-file --cores 16 --max-jobs 1`
succeeded using that cached output. Phase times describe these local runs under
uncontrolled load; they do not establish a benchmark speedup or peak disk use.

## Actual source-only acceptance

`nix develop --command bash experiments/p01-nix/cache-reuse.sh` completed with
fresh inner and outer receipts `0`, in transient service invocation
`08786abbae0f40f19c7af04edc49d1e6`. The service was inactive/collected afterward.
Its fixture is `target/p01-cache-reuse.rIpqrRPf/fixture`: an actual unique comment
was appended to `crates/mailstronaut/src/main.rs`, and all Cargo manifests and
Cargo.lock were checked byte-for-byte unchanged. The fixture was built, rather
than merely evaluated or resolved to an already-built output.

The baseline and variant have exactly the same vendor and dependency derivations
and outputs. Their application source and package derivations differ. The variant
package is `/nix/store/7sa5hmni8z1ywjqnp5fjvwrdl8x6wgs0-mailstronaut-0.1.0.drv`,
output `/nix/store/p68acgxqzfrr8hn489v2r781zbwr21gg-mailstronaut-0.1.0`.
The exact identities are in `artifacts/p01-cache/source-only-{baseline,variant}.json`.
The helper rejected any non-workspace `Compiling`/`Checking` entries; only the four
workspace crates appeared. Servo, SpiderMonkey and the other third-party crates
were reused during build, strict release Clippy and release test compilation.

The variant build took 798 seconds including Nix build/install overhead; the
cached baseline took one second. Its release build, Clippy and test compilation
reported 6m14s, 50.77s and 5m34s. All 35 tests and seven isolated renderer children
passed. These local phase observations are not a controlled speedup benchmark.
`source-only-variant.log`, `source-only-summary.json` and both completion receipts
preserve the result. A live concurrent invocation returned `73` at the build lock,
before starting another package build (`lock-check.log` and `lock-check.status`).

Both installed outputs passed the pinned-shell CLI/native-configuration smoke
with an empty environment and unusable PATH. It exercised version, example and
generated config checks, private initialization modes, refusal to overwrite, and
unsupported-schema rejection. Complete smoke output is in
`baseline-installed-smoke.log` and `variant-installed-smoke.log`. Both complete
runtime closures are preserved; neither contains the dependency artifact output.
This checks the build cache does not become an installed runtime dependency.

The development Cranelift/Wild settings and four scoped LLVM exceptions were
unchanged. Release stays stable LLVM with thin LTO and one codegen unit, and the
package still runs its real format/Clippy/tests. The retained cache costs about
2.2 GiB in addition to package outputs and temporary builds; peak disk/memory use
was not measured. No Nix garbage collection or unrelated cache deletion was run.

## Supporting probes

The two existing source-only Nix probes now use the prepared `cargoVendorDir`
rather than the previous builder's directory convention. They explicitly skip
the compiled dependency archive and Cargo binary-install hook. Both passed in
service invocation `298214431b53467cbfb4385fa822bc83`, receipt `0`:

```sh
nix build --impure --file experiments/p01-nix/nested-metadata.nix \
  --print-build-logs --cores 16 --max-jobs 1 --out-link target/p01-cache-metadata-probe
nix build --impure --file experiments/p01-nix/configure.nix \
  --print-build-logs --cores 16 --max-jobs 1 --out-link target/p01-cache-configure-probe
```

All five nested offline metadata calls passed. The native configure-only probe
produced `config.log` and `config.status`; it did not compile SpiderMonkey.
Outputs are `/nix/store/yf8al6b3gdmzfpr8z013h45daidvzrhh-mailstronaut-nested-metadata-probe`
and `/nix/store/q4a65cm99dwmxym5z4fpln7dsn5p8v0v-mailstronaut-mozjs-configure-probe`.
The archived logs confirm no dependency archive was imported and no Rust crates
were compiled. Pinned Nix formatting passed for both updated expressions.
The obsolete untracked alternate-builder experiment was removed after preserving
its initial result; the normal package is the single supported cache path.

The final pinned-shell ShellCheck, Nix formatting, Bash syntax and actionlint
checks passed. A final normal `nix flake check --no-update-lock-file
--print-build-logs --cores 16 --max-jobs 1` also passed, reusing the exact validated
application derivation and rebuilding only the lightweight format/CI-lint checks.
The final gate/static logs and receipt are archived. Archived logs retain all
output lines with trailing whitespace removed; original local logs remain in
`target` (and the diagnostic's `target/.cache` directory).
