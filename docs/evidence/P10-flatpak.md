# P10 — Flatpak SDK probe on the CI host

This is the optional P01 probe in `experiments/p01-flatpak/`, run on the CI host `ci-py`
(x86_64, 16 cores). It is not a production manifest and does not install an application.

Passing on `ci-py`:

- The four Flathub refs pinned in `sdk-lock.json` are installed under `target/p01-flatpak`
  (Freedesktop Platform, SDK, Rust stable and LLVM 22 extensions, branch 26.08). The installed
  commits match the lock. The installation uses about 4.1 GB.
- Bubblewrap needs unprivileged user namespaces. The host's AppArmor restriction blocked them
  until root added profiles that grant `userns` only to `/usr/bin/unshare` and to the
  Nix-provided bubblewrap binary.
- `preflight.sh` passes: pin validation, Flatpak 1.18.4, the bubblewrap sandbox, and the offline
  SDK tool inventory.
- Staging and `run.sh metadata` pass, and so does the metadata-only `flatpak-builder
  --disable-download --build-only` phase. The SDK supplies rustc 1.98.0 (the project declares
  `rust-version = "1.98"`), LLVM and Clang 22.1.8.
- `run.sh compile` passes under the shared full-gate lock: the release workspace build finishes
  with `Finished release profile [optimized]`, in 3m 24s.
- `runtime-smoke.sh` passes inside the Platform runtime: `mailstronaut 0.1.0`, a valid
  configuration, and no GUI started.

The SpiderMonkey-specific checks were removed from the probe. The current dependency graph has no
`mozjs` crates; the renderer is Blitz, and the SpiderMonkey checks came from an older Servo build.

Not established: the production Flatpak manifest with its finish-args (a Wayland socket, no X11,
no broad home access), an exported and installed application, the GUI running inside the sandbox,
graphics drivers, native Wayland access, the portals (file chooser, secret, notification and
background), aarch64, and the release manifest. The P10 Flatpak row therefore stays unchecked.
