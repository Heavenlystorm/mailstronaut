# Development, CI and distribution

The workspace contains a libcosmic window, a synthetic-only Servo viewport, and
initial configuration/storage components. It is not a usable mail client. Consult
the milestone evidence for the current package/runtime checks; model delivery,
live email and Flatpak acceptance remain planned.

## Nix is the development environment

```sh
nix develop
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings
cargo test --workspace --all-features --locked --offline
nix flake check --no-update-lock-file --print-build-logs
nix build --no-update-lock-file
nix run
```

Before dependencies have been prefetched, Cargo `--offline` can fail; in development
use `nix develop --command cargo fetch --locked` once, explicitly online, then the
commands above. Nix builds must use fixed-output dependency fetches and a locked
offline compilation phase, never uncontrolled build-script downloads. Commit
Cargo.lock and flake.lock together when changing dependency/toolchain pins. Nix
uses git-tracked files for a local git flake; stage new source files before testing
`nix build .` or use an explicit `path:` flake during uncommitted bootstrap work.

The scaffold exports `packages.{x86_64-linux,aarch64-linux}.{default,mailstronaut}`,
`devShells.*.default`, `formatter.*` and flake checks. The development shell uses
rust-overlay's pinned nightly with the Cranelift component, aligned Rust tools
and Wild linking for debug/test. Four SpiderMonkey FFI crates use scoped LLVM
overrides until Cranelift supports their symbol-name marker. The stable Nix
package excludes the nightly-only `.cargo/config.toml` and builds with LLVM,
thin LTO and one codegen unit. Keep that exclusion in future Flatpak builds too.
No rustup installation, separate shell.nix, FHS environment or unpinned tool
download is needed. `rust-version = 1.98` follows mail-parser's requirement; the
x86_64 SDK release compile now passes with Rust 1.98.0. Minimum-version tests,
Clippy and other architectures remain unverified. Exact compilers and limitations
are in P01 evidence.

Debug/test builds use line-only debug information to reduce artifact size while
retaining source locations in backtraces. Incremental compilation stays enabled.
For a session that needs debugger variable/type inspection, set
`CARGO_PROFILE_DEV_DEBUG=full CARGO_PROFILE_TEST_DEBUG=full` before `nix develop`;
changing these settings rebuilds affected artifacts. Release retains LLVM,
thin LTO and one codegen unit.

Coordinate one `CARGO_TARGET_DIR` across agents working on this project, using
the main checkout's `target` directory rather than a new cache per worktree.
Use every available CPU for that build. The development shell defaults Cargo,
CMake and Make to `nproc`; an explicit `CARGO_BUILD_JOBS` overrides that default.
The long-build runners and CI pass the detected CPU count to Nix/SDK builds.
Keep one full package build at a time to avoid competing copies of the graph.
Delete only inactive, untracked build outputs inside the current project when
cleaning up. Do not remove other projects' caches, external shared targets or Nix
store/build directories. A deleted target requires rebuilding its dependencies.

The release package uses pinned Crane `buildDepsOnly`/`buildPackage` artifacts
for third-party dependencies. This Crane pin requires Nix 2.31.2 or later; the
minimal CI image uses 2.34.9. Cargo manifests, the lockfile, toolchain, native
inputs, target and release flags determine the dependency build. The vendor
fetch itself consumes a lock-only source, so an ordinary Rust source edit does
not invalidate it. The final package still runs strict release Clippy and the
actual workspace tests. No remote binary cache or signing credentials are needed.

Keep the writable vendor copy's timestamps and executable modes intact. Crane
normalizes archived artifacts to epoch 1; giving unchanged vendor sources fresh
timestamps makes Cargo rebuild them. Both passes use the same vendor preparation,
including source mappings and the five nested mozjs lockfiles. The nightly
development configuration remains excluded from release sources.

After the normal gate, exercise a real source-only rebuild with:

```sh
nix develop --command bash experiments/p01-nix/cache-reuse.sh
```

The helper holds the shared build slot, creates a project-local disposable
fixture, compares exact dependency identities, builds it, rejects third-party
recompilation in the log, and checks installed CLI/configuration behavior. Its
logs and fixture remain under `target`. It also checks that build artifacts are
absent from the installed runtime closure. Caching does not remove the cost of
compiling/linking workspace crates with thin LTO and one codegen unit. Recorded
acceptance and limitations are in [P01 cache evidence](evidence/P01-cache.md).

The `servo-probe` shell is an alias of the default environment. The optional
`gui-test` shell preserves that environment and adds private-display tools plus
the pinned stock COSMIC theme assets. GUI probes must redirect XDG configuration
and use a private display; never modify the user's theme to test a fixture.

P01 extends the shell/package with the **actual** Servo/libcosmic requirements.
Inspect the chosen Servo release's build configuration and `mach print-env` rather
than running an uncontrolled bootstrap installer in a derivation. Initial expected
native set to verify: pkg-config, CMake, Ninja/Make, Clang/libclang/LLVM/lld, NASM,
Python for upstream build scripts, fontconfig/freetype/harfbuzz, Wayland/libxkbcommon,
X11 libraries where enabled, libGL/EGL/Mesa loader, system certificates, fonts and
DBus tooling for tests. Python/native tools may be build-time dependencies; they
are not application code or runtime ML services. Do not add GStreamer/media codecs,
WebXR, WebGPU-in-mail, Bluetooth, devtools or JIT functionality for an email reader
unless a required upstream feature genuinely needs them and it is documented.

Package application resources, native font/graphics loader paths, desktop entry,
icons, metainfo and translations; do not rely on an ambient host LD_LIBRARY_PATH.
Use normal Nix wrappers/search paths rather than a broadly emulated distro root.
Keep one authoritative source for app version/ID; remove bootstrap version
duplication once packaging gains generated metadata. Run `nix build` and `nix run`
on actual COSMIC/Wayland as well as isolated DBus/headless checks. Evaluation alone
does not prove runtime graphics, activation, portals or accounts.

## All Clippy groups

The root manifest enables all active groups: `all`, `pedantic`, `nursery`, `cargo`
and `restriction`. `all` includes correctness/style/performance/suspicious/complexity;
it does not include the four opt-in groups. Every member inherits `[lints]
`workspace = true`. Rust `unsafe_code = forbid` is also inherited.

Clippy's restriction group contains contradictions (for example return/semicolon
styles), and Clippy itself warns against blanket enabling it. The user explicitly
requested all lints, so retain that group. The only workspace-level exception at
bootstrap is the diagnostic complaining about enabling the group itself. A targeted
`#[expect(clippy::..., reason = "...")]` is allowed when a concrete code path needs
it; never disable an entire group to make a milestone pass. Prefer `expect` so stale
exceptions are detected. Dependency code is outside our lint policy; do not fork
upstreams to fix their style. Document necessary Cargo duplicate-version exceptions
when libcosmic/Servo legitimately differ. CI uses `-D warnings` for our workspace.
See [Clippy's own guidance](https://doc.rust-lang.org/clippy/usage.html).

## Minimal Nix container CI

`.github/workflows/ci.yml` starts from the official minimal `nixos/nix:2.34.9` image
pinned by manifest-list digest, verified against Docker Registry during bootstrap.
It fetches the exact event commit through the GitHub archive API, using the job's
read-only contents token. No Node-based checkout action or Ubuntu package install
is needed inside the Nix container. TLS certificates are verified. All compilers
and dependencies come from the flake. Do not dump the job token or use shell xtrace.

The default job runs `nix flake check --no-update-lock-file --print-build-logs`, which
builds/tests/lints the workspace and checks Nix formatting, then runs the packaged
scaffold as an install smoke check. Minimal image does not mean the full Servo
build closure will be small. Cache pinned dependencies after correctness is proven;
do not require a third-party cache or signing secret to run PR checks.

Nix's build sandbox may be unavailable inside a GitHub-hosted Docker job without
extra namespace privileges. The initial job sets `sandbox = false` **inside this
ephemeral container only**; it never changes the developer's Nix configuration.
This still uses pinned derivations, offline Cargo compile and the outer container,
but is not proof of Nix sandbox isolation. P10 adds a sandbox-enabled build on a
runner that supports it. Do not solve this by adding `--privileged` casually.

Future jobs, still with minimal Nix outer containers and tools from the flake:

| Gate | Execution |
| --- | --- |
| Formatting/lint/unit/store/migrations | Every push/PR; `nix flake check`, offline deterministic fixtures |
| Protocol integration | Disposable local Dovecot/SMTP fixture processes in a private test environment; fixed test cert/credentials; exercise TLS/UID/IDLE/SMTP failures, no external accounts |
| GUI/Servo/privacy | DBus session + headless compositor/software GL from Nix, synthetic mail/resource traps; save screenshots/logs; separate real COSMIC hardware acceptance |
| Classifier | Small committed reference tensors/fixtures on each change; model-artifact CPU experiment when model/policy changes, cache by exact artifact hashes |
| Nix package | x86_64 native initially; aarch64 on a matching runner before declaring that release architecture supported |
| Flatpak | Minimal Nix container supplies flatpak/flatpak-builder/appstream validation; Flatpak SDK is the controlled inner build environment, not a second CI base image |

Flatpak/bubblewrap often needs user namespaces not available in hosted job containers.
P10 must probe this on the actual CI runner and use a configured dedicated runner
with documented minimal capabilities if necessary. If no such runner is available,
leave full Flatpak CI unchecked; JSON/schema validation alone is not a Flatpak build.
Do not claim a generated bundle is installable without an install/start/portal test.
Upload artifacts through a supported Nix-supplied tool or a separately justified
action runtime; adding Node solely for artifact upload is optional infrastructure,
not part of the client. Do not execute arbitrary untrusted code with release secrets.

## Flatpak package target

Application ID: `io.github.eduardz1.Mailstronaut`; binary: `mailstronaut`. Keep the
same ID for desktop entry, icon, metainfo, portal identity and activation. A future
unstable channel must have an explicit separate data/secret migration policy;
do not silently share destructive migrations across stable/beta installs.

Use `flatpak-builder` with a Freedesktop runtime/SDK (no GTK/GNOME dependency just
for libcosmic). P10 must select and pin a supported branch (evaluate 26.08 first,
then 25.08 if dependency compatibility requires it), SDK commit and Rust extension
matching at least the selected MSRV. The P01 experiment now verifies a
project-local x86_64 SDK 26.08 installation with Rust 1.98.0 and the LLVM 22
extension. Offline workspace release compilation and the probe binary's CLI/native
Nix helper pass in the Platform runtime. Production Flatpak installation, graphics
and portals remain **unverified**.
See [the experiment](../experiments/p01-flatpak/README.md) and its SDK commit lock;
it is not the production release manifest. Pin upstream git sources including
submodules and source checksums. Generate Cargo source manifests from Cargo.lock
using standard tools, and lock generated output. The early spike reuses Nix's
hashed, source-separated vendor output because plain Cargo vendor rejects the
two same-version `dpi` packages from different sources. Validate any replacement
generator against this graph, including submodules and nested mozjs metadata.
All builds use `cargo --frozen --offline`; no network access in build commands.
Provide upstream-generated/prebuilt SpiderMonkey artifacts as explicit hashed
sources if required; never let a build.rs download an untracked binary.

Initial finish permissions to test, no broader:

```text
--share=network             IMAP/SMTP/OAuth and explicitly requested resources
--socket=wayland            COSMIC native display
--socket=fallback-x11       optional X11 fallback
--share=ipc                 only if required for that X11 fallback
--device=dri                Servo/libcosmic graphics
```

Portals handle file chooser/document access, external browser, notification,
background and Secret storage via ASHPD/oo7. Let Flatpak's own application bus name
ownership work normally; no unrestricted session/system bus. If a missing COSMIC
portal requires an additional narrow D-Bus permission, document/test it explicitly;
do not preemptively grant `org.freedesktop.secrets`, whole-session access,
home/host filesystem, microphone/camera, host spawning or all-device access.
Network permission applies to the process, so the renderer's default-deny policy
remains essential; Flatpak alone does not stop an email from requesting a URL.

The app owns only its sandbox's XDG directories and portal-granted files. HM direct
Nix-store symlinks are handled as documented in CONFIGURATION.md, without a broad
filesystem permission in the normal package. Native Nix and Flatpak data/credential
namespaces differ; an explicit future migration can copy selected mail/settings,
but do not promise automatic shared data or simultaneous writers.

Required packaging outputs in P10: desktop entry with mailto handler and
DBusActivatable consistent with the implemented activation service, icon at standard
sizes, AppStream metainfo with accurate features/license/privacy/URLs, translations,
exact source archive, Nix closure/checks and `.flatpak` bundle/repository. Define
desktop/portal activation and headless lifetime before enabling DBusActivatable;
setting that key without a working service makes launches disappear.

The repository currently has **no production Flatpak manifest or release bundle**.
Adding a guessed manifest before the Servo/native dependency graph is proven
would give a false sense of distributability. P10 provides the concrete recipe,
build validation and installed-runtime acceptance for both channels before v0.1.

## Release evidence

Before tagging: exact-head CI green; clean native Nix and Flatpak builds; compiler,
Cargo/Nix locks/native sources/model manifest recorded; dependency/license audit;
DB migration backup/recovery tested; no secrets/real mail in artifacts; source and
notice availability. Check software-only graphics and reference hardware.

On COSMIC exercise theme switching, keyring unlock, default browser callbacks,
attachments via portal, mailto handler, single-instance window reopen, notifications
after last-window-close, permission refusal, suspend/resume, time jumps and two
simultaneously installed distributions. Live test each of the five requested
providers/account types with the user's authorized test accounts; use controlled
mail for sending/deleting and never send mail merely to prove login.

Local compilation is not CI success, account login, Google/Entra application
verification, portal support, Flathub acceptance, or a released package. Record
each separately in PLAN.md and evidence. Publishing to registries/Flathub or
changing the user's Home Manager configuration is outside this planning task.
