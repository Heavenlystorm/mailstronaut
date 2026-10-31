# Development, CI and distribution

The active stack is GPUI Kit + Blitz. Existing mail/storage/configuration code is
retained; old toolkit/browser runtime evidence does not establish acceptance of
this package. The current checklist is [PLAN.md](PLAN.md).

## Nix development and package

```sh
nix develop
cargo fetch --locked                 # explicit initial dependency download
bash ci/check.sh                      # format, shell/CI lint, strict Clippy, tests
nix flake check --no-update-lock-file --print-build-logs
nix build --no-update-lock-file
nix run .# -- --demo
```

The flake pins Nixpkgs, rust-overlay and Crane. Development uses matching nightly
Rust tools with Cranelift and Wild for debug/test. Release uses the pinned stable
LLVM compiler, thin LTO and one codegen unit; `.cargo/config.toml` development
settings are excluded from package sources. The old browser's LLVM exceptions,
assembler overrides, hardening changes and nested metadata hooks are retired.
Scoped Pulp 0.22.3 and fearless_simd 0.4.1 LLVM debug/test overrides handle
exact assembly link names and an unsupported AVX2 conversion intrinsic respectively.
Vello Common/CPU also need LLVM because they instantiate those inline SIMD kernels;
changing only fearless_simd still aborts at runtime.
AnyRender Vello CPU also instantiates a SIMD premultiplication kernel for images:
its debug/test LLVM override resolves the demonstrated unsupported
`llvm.x86.avx2.psrl.w` intrinsic. Release settings are unchanged.
Other crates retain Cranelift and all debug/test linking uses Wild. Remove each
exception when pinned Cranelift supports the demonstrated operation.

The shell defaults Cargo/CMake/Make jobs to `nproc`; explicitly set
`CARGO_BUILD_JOBS` to override. Coordinate one full release build and one shared
project-local target rather than duplicate caches. Retain line-table debug data
and incremental builds. Only clean inactive outputs inside this project; never
clean unrelated projects, external shared caches or Nix store/build directories.

Crane reads source mappings from the immutable Nix vendor output. Its standard
assembly exposes a resolved root `config.toml`; both passes use the same store
source paths. Do not copy sources into temporary build directories: unsandboxed
CI uses different directories for each derivation and invalidates Cargo artifacts.
Its dependency source is lock-only; both dependency and package
passes use identical toolchain, native inputs, target and profile. Final strict
Clippy and actual tests remain mandatory. Revalidate source-only artifact reuse
for changes to the graph or builder. The project-local probe
`nix develop --command bash experiments/p01-nix/cache-reuse.sh` changes observable
CLI behavior in a disposable source-only
fixture, rejects third-party recompilation in either real package build, and checks
tests, installed configuration and runtime closure. For CI-equivalent validation,
set `CACHE_REUSE_SANDBOX=false` with a store that permits disabling sandboxing.
The probe rejects an ignored setting before compiling; sandboxed local results
do not prove reuse across CI's different temporary paths. Old cache results do not
transfer automatically.

The standard vendor staging output includes `Cargo.lock` verbatim, including
workspace dependency references. Refresh `cargoHash` from a fresh staging build
after lockfile changes; an existing fixed-output cache can mask a stale hash.
`ci/nix-check.sh` compares raw workspace/vendor lockfile hashes before compilation
and rejects that case without adding tools to the minimal container.

The immutable Snix source archive under `third-party` preserves the exact pinned
upstream Git commit/tree when its forge is unavailable. Cargo revision and fixed
vendor hash remain authoritative. This is a source cache, not an evaluator fork.
Compilation is locked/offline, with no build-script downloads. Stage new files
before package checks because local Git flakes exclude untracked files.

Native inputs include pkg-config, Clang/libclang, CMake/Ninja as needed by actual
dependencies, Wayland/libxkbcommon, fontconfig/freetype, Vulkan/GL loaders, OpenSSL,
zstd and the XCB libraries required by GPUI's published compiled platform graph.
Application startup selects only Wayland. There is no embedded X11 surface or
XWayland fallback. Use normal Nix search paths/wrappers rather than an FHS root.
Embedded GPUI Kit assets avoid icon downloads at runtime.

`devShells.*.gui-test` adds Sway, Grim and private-session tools; it does not launch
XWayland. GUI tests redirect all XDG paths into a project-local/private profile.
Use a nested/headless Wayland compositor and explicitly unset `DISPLAY`. Capture
actual window screenshots, compositor/client protocol and failures; a successful
compile is not native Wayland or accessibility proof. Do not modify the user's
desktop theme/configuration to test synthetic fixtures.

## Home Manager

The application directly evaluates `$XDG_CONFIG_HOME/mailstronaut/config.nix` in
a bounded helper using Snix/Serde. Home Manager places that exact `.nix` file:

```nix
home.packages = [ inputs.mailstronaut.packages.${pkgs.stdenv.hostPlatform.system}.default ];
xdg.configFile."mailstronaut/config.nix".source = ./config.nix;
```

There is no module converting a declarative schema into another file format.
The planned optional `homeManagerModules.default` only adds package/native-file
lifecycle: native `settings`, immutable by default, or opt-in `mutableSettings`.
It must not mirror every Rust option. Keep direct-file composition supported.
Mutable activation reapplies declared values while retaining undeclared UI edits;
Settings exports a secret-free native Nix snapshot for review/import. These are
pending P02/P10 tasks, not current flake exports. See CONFIGURATION.md for ownership,
precedence, collision handling and export/reimport acceptance. Test first/repeated
activation and ownership changes in disposable profiles. Never put passwords/tokens
in Nix configuration or activate a user's Home Manager profile as a build test.

## Lints and CI

Root Clippy policy is `all`, `pedantic`, `nursery`, `dbg_macro`, `todo`,
`unimplemented`, `unwrap_used`; members inherit it and CI applies `-D warnings`.
Unsafe workspace code is forbidden. No blanket restriction/cargo lint group.
Use reasoned scoped expectations for demonstrated exceptions, not broad disables.

CI uses the official minimal `nixos/nix:2.34.9` container pinned by digest. It
fetches the event's exact source with read-only contents access, then invokes
`ci/nix-check.sh`: bounded dependency-fetch retries, one flake gate and installed
CLI/config smoke. Build tools/dependencies come from Nix; no host package-manager
bootstrap or Node checkout action is needed. A minimal container does not guarantee
a small build: track disk pressure, lock graph size and dependency artifacts.
Never delete unrelated host files to make CI pass.

Pass the explicit `.#` target to `nix run` before the application's `--`
separator. Nix 2.34.9 otherwise interprets `--version` as a flake path. The installed
smoke checks both the version and the native Nix configuration helper without a
display.

Record local development, release/package, hosted CI, private credential fixture,
Wayland GUI, accessibility, live-account and Flatpak evidence separately. Retain
behavior tests for canonical storage, MIME, threading, TLS, authentication,
delivery uncertainty and explicit seen semantics as frontend code changes.

## Flatpak

Flatpak remains required but is not currently accepted for the replacement stack.
Select a supported Freedesktop runtime/Rust SDK branch, pin the exact SDK and use
locked generated offline Cargo sources, including the exact Snix source. Build
our application entirely in Rust; native upstream build tools remain build tools.
Do not turn an obsolete browser SDK probe into a distributed manifest.

Start with `--socket=wayland`, `--device=dri`, `--share=ipc` and `--share=network`
(the last is needed for mail protocols, never automatic HTML resources). Do not
add X11/fallback-X11, home/host filesystem access, unrestricted session/system bus,
or expose the user's Nix installation. Use app-private data and portals for
files, external links, encrypted secret storage, notifications and optional
background/autostart. Verify actual portal permissions/lifecycle and account TLS
inside the sandbox, plus native Wayland with no XWayland. If an upstream component
requires an extra permission, demonstrate and document the narrow need first.

P10 must produce a real manifest, metainfo/icons/desktop entry and installed
bundle, then exercise offline/online mail, font/HTML rendering, configuration,
OAuth browser callback, keyring, attachments, notifications and clean shutdown.
A package build does not prove these runtime workflows. No live credentials,
private mail or model downloads belong in CI artifacts.
