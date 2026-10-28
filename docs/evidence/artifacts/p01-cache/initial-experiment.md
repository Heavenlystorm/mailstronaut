> Historical snapshot of the first failed attempt. The experiment paths and
> default-builder statements below describe that attempt before the cache fix.
> The corrected implementation and final acceptance are in ../../P01-cache.md.

Historical record of the first attempt, before the timestamp correction and normal package integration. See ../../P01-cache.md for the subsequent result.

# Crane dependency-artifact experiment

This is a failed cache experiment, not an adopted package. Base HEAD was
`2ceb35d6f7e591306ce28db5bfd473a414880192`. Crane v0.24.0 is pinned at
`692f7e9ef2ece8125b466f66f2af532b3edaed0d`. The normal package remains
`buildRustPackage`. The experiment is `crane-experiment.nix`; it reuses the
standard vendor output, prepares a writable copy in both passes, and uses
Crane's `buildDepsOnly` and `buildPackage`.

The coordinated transient user service `mailstronaut-crane-experiment`,
invocation `ab90a591ac1049cb95752f6f6dd46761`, took the existing
`target/p01-package-gate.lock` and used `--cores 16 --max-jobs 1` with
`CARGO_CACHE_AUTO_CLEAN_FREQUENCY=never`. Its fresh status was `exit=0`,
`elapsed_seconds=2848`; the unit was subsequently inactive. Full log and
runner: `target/.cache/crane-experiment/build.log` and `run.sh`; fresh status:
`target/.cache/crane-experiment/status`.

- Dependency derivation: `/nix/store/66dqhwzhbn12msk8wga60amjhcricjgf-mailstronaut-deps-0.1.0.drv`.
  The retained compressed artifact output is
  `/nix/store/b1i2z5s7blgvaf6cik32snp35dlvz97h-mailstronaut-deps-0.1.0`,
  `du -sh` 2.2 GiB. Its release check took 9m 01s, release build 8m 53s,
  and dummy test compilation 7.63s.
- Experimental package derivation:
  `/nix/store/5wnxfmk3vgzd9b38la3lyj0b0cx7sr29-mailstronaut-0.1.0.drv`;
  output `/nix/store/s4r0zdlsna44s2172s89rllylxhzpwzv-mailstronaut-0.1.0`.
  Its release build took 20m 41s, strict release workspace all-target/all-feature
  Clippy 2m 36s, and release test compilation 4m 57s. All 35 workspace tests
  and seven isolated renderer child invocations passed.
- **Reuse failed:** after the package imported the dependency artifact, its
  build log shows `Compiling harfbuzz-sys`, `mozjs_sys`, `mozjs`, and
  `servo-script`, among other dependencies. The cause was not established;
  writable-copy timestamps are a hypothesis only. No speedup is claimed.
- A disposable fixture under `target/.cache/crane-experiment/fixture` was
  made from HEAD and changed only by appending a Rust comment. Independent
  evaluations, without fixing an artifact pointer, gave different dependency
  derivation paths (`66dq...` baseline and `rii1...` fixture), but the same
  dependency output path `b1i2...` and vendor output `p0k...`. This does not
  prove a successful source-only build or the requested stable derivation
  identity. The fixture was not built after the baseline already failed reuse.
- The pinned-shell installed smoke passed against the experimental output;
  `target/.cache/crane-experiment/installed-smoke.log` records it. The pinned
  `nixfmt --check` passed on the flake and experiment. `nix flake check
  --no-build --no-update-lock-file` evaluated x86_64 outputs; it is not a
  build, package gate, or CI proof. A normal flake check/build was not rerun:
  it would be another full package build, and this candidate already failed
  the cache acceptance criterion.

The filesystem had 179 GiB free before the run, 170 GiB at a sampled point,
then 175 GiB after; these are global snapshots, not isolated peak disk usage.
A sampled system memory reading reached 60 GiB used excluding cache, and
swap reached 7.4 GiB; neither is a measured peak. No GUI, portal, live account,
aarch64, remote CI or Flatpak acceptance is claimed.
