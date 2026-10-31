# Mailstronaut implementation instructions

This repository currently contains a planning baseline and an executable scaffold,
not a working email client. The user asked for architecture first. The next feature
task starts with docs/PLAN.md P01 unless the user explicitly changes that order.

Read docs/ARCHITECTURE.md, docs/DEPENDENCIES.md and the document for your milestone
before editing. Preserve the user's requirements: Rust application code, GPUI Kit + Blitz,
native Wayland and component light/dark themes, local-only lightweight classifier, native Nix configuration,
Home Manager direct use, Nix/Flatpak distribution, and minimal Nix-container CI.

- Develop and run checks through `nix develop`; do not use ambient Cargo/Rust as
  evidence for the pinned build. `nix flake check` is the package/check gate.
- The development shell selects pinned nightly Rust with Cranelift for debug/test
  builds and Wild for linking. Release uses LLVM, thin LTO and one codegen unit.
  Coordinate full Nix release builds instead of duplicating them across agents.
- Remove obsolete browser/toolkit build hooks when their dependencies are retired.
  New compiler/backend exceptions require a concrete diagnostic and narrow scope.
- Root Cargo.toml owns workspace metadata, dependencies and all lint groups.
  Members use `workspace = true` for dependencies/metadata/lints. Keep
  `members = ["crates/*"]`. Create planned crates only when implemented.
- After Cargo.lock changes, validate cargoHash against fresh vendor staging, even
  for workspace dependency references: staging includes Cargo.lock itself. A
  successful cached build is insufficient. `ci/nix-check.sh` compares workspace
  and vendored lock hashes before compilation to detect this stale-cache case.
- Keep the root Clippy policy strict and practical: `all`, `pedantic`, `nursery`
  plus selected checks for debug macros, unfinished code and unguarded unwraps.
  Do not enable the entire restriction or cargo group. Use narrow reasoned
  `#[expect]` only for demonstrated exceptions.
  Unsafe code in our workspace is forbidden. Avoid forks of large dependencies.
- Prefer the specified existing libraries. Do not reimplement MIME, TLS, OAuth,
  Nix evaluation, an HTML engine, a scheduler daemon or transformer architecture.
- Before custom UI, check GPUI Kit's existing components across the full catalog.
  Follow docs/PLAN.md's per-subproblem choices and docs/INTERFACE.md's API mapping
  and pin limitations; Sidebar/Settings/Attachment are examples, not the entire list.
- After each implemented feature, show an actual current-app screenshot in chat
  from a private native Wayland profile with synthetic mail. Reference screenshots
  stay in ignored target/design-references and must not be committed or shipped.
- P01/P02 are feasibility gates. No fake HTML pane, alternative engine, plaintext
  secret fallback, hidden TOML config, remote classifier or untracked model fetch.
- Never fetch remote email resources by default, execute email scripts, log
  credentials/bodies, or include real mail in tests. Model input is untrusted data.
- Check off PLAN.md rows only with actual tests/artifacts and a short evidence
  record. Distinguish local checks, CI, GUI/portal/runtime, live auth and distribution.

Routine scaffold checks:

```sh
nix develop --command bash ci/check.sh
nix flake check --no-update-lock-file --print-build-logs
nix build --no-update-lock-file
```

Once implemented, add milestone-specific tests from PLAN.md. Tests must establish
behavior, not merely repeat implementation internals. Keep evidence honest about
unrun remote or user-session acceptance.
