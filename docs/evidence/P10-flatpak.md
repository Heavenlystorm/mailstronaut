# P10 — Flatpak build on the CI host

The current source archive `45bebbe1…` contains search-filter source `54f2a91`.
Its November 10 build/install, private runtime walkthroughs and Nix checks passed
as recorded below. Earlier November 5 receipts are retained as history, not proof
for the newer source.

Two steps ran on `ci-py` (x86_64, Ubuntu, 16 cores): the optional SDK probe in
`experiments/p01-flatpak/`, and then the production manifest in `flatpak/`. Both use the
Flathub refs pinned in `experiments/p01-flatpak/sdk-lock.json`, installed into an isolated
installation under `target/`.

## SDK probe

- The four pinned refs (Freedesktop Platform, SDK, Rust stable and LLVM 22 extensions, branch
  26.08) are installed, and their commits match the lock. The installation uses about 4.1 GB.
- Bubblewrap needs unprivileged user namespaces. The host's AppArmor restriction blocked them
  until root added profiles that grant `userns` only to `/usr/bin/unshare` and to the
  Nix-provided bubblewrap binary.
- `preflight.sh` passes: pin validation, Flatpak 1.18.4, the sandbox, and the offline SDK tool
  inventory.
- The metadata-only builder phase passes with rustc 1.98.0 (the project declares
  `rust-version = "1.98"`), LLVM and Clang 22.1.8.
- `run.sh compile` passes under the shared full-gate lock: `Finished release profile [optimized]`
  in 3m 24s.
- `runtime-smoke.sh` passes in the Platform runtime: `mailstronaut 0.1.0`, a valid configuration.

The SpiderMonkey-specific checks were removed from the probe. The current dependency graph has no
`mozjs` crates; the renderer is Blitz, and the checks came from an older Servo build.

## Production manifest

- `flatpak/io.github.eduardz1.Mailstronaut.json` targets the Platform 26.08 runtime and the SDK
  with the rust-stable and llvm22 extensions from the lock. Its finish-args are
  `--socket=wayland`, `--share=network` and `--device=dri`. It requests no X11 socket and no
  broad home access.
- Its two sources are archives pinned by SHA-256. The application source archive is
  `28213efd…c295` (367 KB), and the vendor archive is `0a7438ab…5582` (131 MB). Both were
  created on `ci-py` from the staged Nix package source and `cargoDeps`. The archive paths are
  local, under `target/`, so the pins hold for that host's staging.
- `flatpak/build-app.sh` builds offline with `cargo build --frozen --offline --release --package
  mailstronaut`. It installs only the binary. The module finished in 3m 24s, exported the
  application, and installed it into the isolated installation. The build ran under the shared
  full-gate lock.
- `flatpak run --command=mailstronaut --unshare=network io.github.eduardz1.Mailstronaut --version`
  prints `mailstronaut 0.1.0`.
- `config check` of the examples configuration inside the installed app reports a valid
  configuration. The examples folder was mounted read-only for that check only; the manifest does
  not grant it.
- The installed application opens its main window inside the sandbox, in the private headless
  Wayland session, with network unshared. The capture shows the empty mailbox state with no
  accounts (`flatpak/gui-smoke.sh`, through the `MAILSTRONAUT_FLATPAK_GUI_BINARY` switch of
  `credentials.sh`).
- The application ships a desktop entry, a metainfo file and its icon from `assets/icon.svg`.
  `desktop-file-validate` passes. `appstreamcli validate --no-net` passes, with one informational
  note: no developer element is declared, which is left for the maintainer to decide. The export
  composes the AppStream data and exports all three files. The composition uses the Nix
  `appstream` tool, because the host's copy loads a different glibc.
- The rebuilt installation still reports `mailstronaut 0.1.0`, a valid configuration, and a window
  in the sandbox.
- Portal reachability: a process inside the sandbox reads the FileChooser (version 4) and
  Notification (version 1) portal properties, served by the protocol-accurate fixture on the private
  bus. The request goes through the sandbox's D-Bus proxy (`flatpak/portal-reach.sh`, through the
  `MAILSTRONAUT_FLATPAK_PORTAL_BINARY` switch of `credentials.sh`).
- Sandboxed account and composer (`flatpak/attach-smoke.sh`, through the
  `MAILSTRONAUT_FLATPAK_ATTACH_BINARY` switch of `credentials.sh`). The installed application opens
  its keyring with the key from the Secret portal, which the private fixture serves
  (`org.freedesktop.portal.Secret`). The synthetic account "Synthetic sender" becomes active, the
  composer saves a draft, and the FileChooser portal attaches a synthetic 37-byte file that the
  sandbox reads. The run passed in the private headless session with network unshared. Its capture
  shows the composer with the attachment and the status "Saved locally."
- The Flatpak module archive was regenerated from the final tree, and its pin in the manifest was
  updated to match. The vendor archive, whose lock file is unchanged, keeps its pin.
- Automatic OAuth refresh on a file-backed store is refused before any provider contact. The
  regression test `auth::tests::automatic_refresh_on_a_file_backed_store_is_refused_before_provider_contact`
  passes. With the fence temporarily removed, the same test fails with `OAuth(Network)`, which shows
  that the token endpoint is reached without the fence.

## Current-source acceptance (2026-11-05)

- The production manifest builds the Flatpak from the source archive of commit `ffb85b4`. Its
  SHA-256 `b8c9c313…daeee9` is the pin in the manifest, and the archive excludes `flatpak/`. The
  build ran under the shared gate lock, exited 0 and installed `app/io.github.eduardz1.Mailstronaut`.
- `flatpak/gui-smoke.sh`: the installed application opened its window in the sandbox with network
  unshared.
- `flatpak/portal-reach.sh`: the sandbox reached FileChooser (version 4) and Notification (version 1)
  through the D-Bus proxy.
- `flatpak/attach-smoke.sh`: the portal-backed keyring holds the synthetic password. The sandboxed
  account composed a draft and saved it, and the FileChooser fixture attached one synthetic file. The
  capture shows the attachment and the status "Saved locally."
- In the same invocation, the private Secret Service round trip and the fail-closed plaintext check
  pass.

Scope: the Secret and FileChooser portals are protocol-accurate fixtures on the private bus, not the
desktop's portals. The FileChooser fixture returns host paths, so the document portal is not
exercised. The Background portal, real-session Wayland and the graphics path are not exercised.

## Reproducible production inputs (2026-11-10)

`nix develop .#flatpak` now supplies the production tooling from the existing Nix
lock. The older optional probe shell delegates to it. `flatpak/prepare.sh` realizes
the existing Nix `src` and `cargoDeps`, compares the workspace/source/vendor
lockfiles, resolves the standard vendor placeholder for the SDK, and produces
app/vendor archives with normalized metadata. It does not invent a vendor layout
or run a separate dependency resolver. `build-and-install.sh` runs preparation and
SDK preflight before the offline build, retaining its fresh numeric status receipt.
The obsolete Servo assembler override was removed from production and probe builds.

On `ci-py` through the pinned shell, Nix formatting and ShellCheck passed. Normal
preparation refused the old pins with exit 1 and left the manifest unchanged.
Explicit `--update-pins` generated these hashes; another fresh preparation reproduced
both byte for byte and exited 0:

| Archive | SHA-256 |
| --- | --- |
| app-source.tar.xz | `45bebbe13581a2e7bfc3fd999fd48396d1d42bbd89078ba7f83207bc3b62df5f` |
| vendor.tar.xz | `10dde9661971815b10bdc3f2b08d3f984b57122d926d5f1cfbcfd8d78d452e57` |

Both archived Cargo.lock files matched the checkout. Preparation retained 118 MiB
of compressed inputs, without temporary source copies. A 64 MiB sample of the
actual vendor stream compressed identically with one threaded worker (`+1`) and
automatic multicore workers (`0`), using fixed 24 MiB blocks. The tool ignores
ambient TAR/XZ options and forbids encoder adjustments that could change output.
This uses [XZ's documented threaded mode](https://tukaani.org/xz/man/xz.1.html),
not an application compressor. Full ignored logs:
`target/remote/flatpak-preparation.log` and `target/remote/flatpak-thread-repro.log`.

The four SDK refs were restored into the isolated project installation and rolled
to the exact existing `sdk-lock.json` commits. Preflight passed with the installed
SDK compiler/LLVM inventory and fonts. The refs occupy 4.1 GiB; root/data free space
was 32/24 GiB after provisioning. No unrelated files or caches were deleted.
The final Nix gate for the complete packaging changes passed with exit 0 and
`all checks passed!`, including the added production ShellCheck targets and optional
shells. It reused the unchanged, previously tested Rust package. aarch64-linux was
explicitly omitted. [Compact receipts](artifacts/p10-flatpak-workflow/checks.log).

## Current-source acceptance (2026-11-10)

`nix develop .#flatpak --command bash flatpak/build-and-install.sh` completed under
the shared package-gate lock. Its current numeric status is `0`; the full log
records a 3m 29s release compilation, export and installation. The build used the
exact SDK pins above, LLVM/thin LTO/one codegen unit and all available build jobs,
with downloads disabled for compilation. It required no assembler override.
The installer also installed the normal related GL/locale/codec/debug refs; these
are runtime extensions, not additional locked SDK/compiler inputs.

`nix develop .#gui-test --command bash flatpak/check.sh` exited 0. This reusable
driver discovers the native fixture executables from Cargo's machine-readable
output and runs the existing checks with network unshared:

- The installed app reports `mailstronaut 0.1.0` and validates the explicitly mounted
  examples configuration through its embedded native evaluator.
- Its native Wayland window appears in the private headless session.
- The sandbox reaches the FileChooser v4 and Notification v1 fixture portals through
  the D-Bus proxy.
- Its Secret-portal-backed keyring activates the synthetic account; the composer
  saves a local draft and attaches a file returned by the private FileChooser.
- Private native Secret Service isolation, lock/failure handling and the check that
  persisted synthetic credentials do not appear in plaintext also pass.

The current private screenshots are under `target/p02.XhUy5f/flatpak-gui/` and
`target/p02.XhUy5f/flatpak-attach/`, including `flatpak-attached.png`. Complete
ignored logs/receipts: `target/p10-flatpak-build.log`,
`target/p10-flatpak-build.status`, `target/remote/flatpak-current-acceptance.log` and
`target/remote/flatpak-workflow-land-gate.log`. The final isolated installation is
5.5 GiB and production build/archive state 326 MiB. Root/data free space remained
32/23 GiB. No unrelated caches were collected or deleted.

The Ubuntu install logged a nonfatal missing NixOS hicolor `index.theme` path.
Export/install and the application window succeeded; real desktop icon integration
remains part of the desktop acceptance below. These private fixtures do not prove
the desktop's own Secret/FileChooser/document/Background portals or a real GPU path.

## Not established

- Native Wayland on a real desktop session (only the headless sway session was used), and the
  graphics path: `--device=dri` is declared, but which renderer the window used was not checked.
- The Secret and FileChooser portals were exercised only through the protocol-accurate fixture on the
  private bus, not a real desktop portal. The fixture returns host paths, so the document portal that
  would return a sandbox path for a chosen file is not exercised.
- Notifications sent from the sandbox, and the Background portal.
- The developer element in the metainfo file, and a screenshot entry for the store listing.
- Release signing, a published repository, aarch64, and the final release manifest.

The P10 Flatpak row therefore stays unchecked.

## Local SDK and installed runtime recheck

The x86_64 source at `722472b` built offline under the four pinned Freedesktop
26.08 refs, including the Candle and USearch dependency graph. Rust 1.98 compiled
the release in 7m 25s. The first export failed after compilation because the
local FUSE mount was denied. Flatpak Builder's `--disable-rofiles-fuse` reused
the completed module, composed AppStream metadata, and exported app commit
`7880db2efa359322f8109f65b69cb86b13da51cd594c594df1b983f58ace54fe`.

The app installed from the local repository with the verified runtime already
present. `flatpak/check.sh` passed against that installation (`flatpak-acceptance-exit=0`).
The no-network runs validated `--version`, the example Nix declaration, a headless
Wayland window, FileChooser and Notification portal reachability, and native
Secret-backed credential lifecycle. The sandbox activated a synthetic account,
saved a draft, and attached a synthetic file returned by the private FileChooser.
The fixture screenshots are in `target/p02.tY1w5y`. No real portal, live provider,
GPU acceleration, or store listing is verified by these runs.
