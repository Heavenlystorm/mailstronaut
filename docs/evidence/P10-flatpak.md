# P10 — Flatpak build on the CI host

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
- A check that opened the composer and attached a file through the chooser inside the sandbox did not
  reach the attach step. The configured account stayed inactive, because the application reads its
  password from the system keyring, and the sandbox has no route to the keyring. The application has no
  Secret portal path yet, so that check was removed rather than recorded as passing.

## Not established

- Native Wayland on a real desktop session (only the headless sway session was used), and the
  graphics path: `--device=dri` is declared, but which renderer the window used was not checked.
- The Secret portal: the application does not read its keyring password from inside the sandbox yet,
  so an account cannot activate there.
- The file chooser flow itself inside the sandbox, the document portal that would return a sandbox
  path for a chosen file, notifications sent from the sandbox, and the Background portal.
- The developer element in the metainfo file, and a screenshot entry for the store listing.
- Release signing, a published repository, aarch64, and the final release manifest.

The P10 Flatpak row therefore stays unchecked.
