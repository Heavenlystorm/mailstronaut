# P01 Flatpak SDK experiment

This is a build-environment probe, not a production Flatpak package. The checked
manifest runs **metadata only**. No application is installed or exported. Current
results and limits are recorded in [P01 evidence](../../docs/evidence/P01.md).

All Flatpak installations, SDKs, caches, sources and build output live under
`target/p01-flatpak`. `isolated.sh` selects that installation through
`FLATPAK_USER_DIR` and isolates the system-installation path and XDG directories.
Use it for every Flatpak command below; do not add remotes to the host installation.
The optional development shell adds tools from the existing Nixpkgs lock:

```sh
XDG_CACHE_HOME="$PWD/target/nix-cache" nix develop --impure --expr \
  'import ./experiments/p01-flatpak/shell.nix'
```

Inside that shell, provision the signed build dependencies once:

```sh
bash experiments/p01-flatpak/isolated.sh flatpak remote-add --user \
  --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
bash experiments/p01-flatpak/isolated.sh flatpak install --user --noninteractive \
  --no-related --no-deps flathub \
  org.freedesktop.Platform/x86_64/26.08 \
  org.freedesktop.Sdk/x86_64/26.08 \
  org.freedesktop.Sdk.Extension.rust-stable/x86_64/26.08 \
  org.freedesktop.Sdk.Extension.llvm22/x86_64/26.08
bash experiments/p01-flatpak/isolated.sh bash experiments/p01-flatpak/preflight.sh
```

`preflight.sh` requires exactly the four valid commit pins for x86_64/26.08,
rejects installed commits that differ from `sdk-lock.json`, checks
user namespaces/bubblewrap, and runs an offline SDK sandbox inventory. An updated
remote branch is not permission to update the lock silently. To reproduce an older
available signed commit, use `flatpak update --user --commit=COMMIT REF` through
the same isolated wrapper, then rerun preflight. Architecture acceptance is
x86_64 only. No host Home Manager configuration or Flatpak application is activated.

Stage the actual Nix package source and its standard source-separated vendor tree:

```sh
bash experiments/p01-flatpak/stage.sh
bash experiments/p01-flatpak/isolated.sh bash experiments/p01-flatpak/run.sh metadata
```

The package's `src` and `cargoDeps` must already be realized. Staging aborts rather
than overwrite a nonempty source directory. It checks all three lockfiles agree,
omits the nightly-only development `.cargo` configuration, and copies dependencies
without writable hardlinks into the Nix store. The app and vendor directories are
siblings: placing vendor inside the app causes Cargo to mistake the vendored crates
for workspace members. `cargo fetch --frozen --offline` checks the workspace.

The Nix vendor separates registry/Git sources, including the two `dpi 0.1.2`
packages and libcosmic's Iced sources. Plain Cargo vendor rejects duplicate
name/version pairs from different sources. This experiment reuses the already
hashed Nix vendor instead of implementing another generator. Its `@vendor@`
placeholder is resolved for the SDK path.

To exercise the manifest's metadata phase through the real builder:

```sh
bash experiments/p01-flatpak/isolated.sh bash experiments/p01-flatpak/run.sh builder-metadata
```

This checks the installed SDK pins immediately before invoking the builder with
`--disable-download --build-only`. It requires a fresh `metadata-build` destination;
it does not remove an existing one. Do not use `--stop-at=mailstronaut`: it skips
the application module. The builder
copies sources into its own build directory; `run.sh` reuses the staged tree to
avoid that additional copy for later direct SDK work.

After the coordinated Nix release gate has finished, the next acceptance command
is the explicit `run.sh compile` phase through the isolated wrapper. It runs the
same metadata checks, then builds the whole workspace with one job per available
CPU, the stable
SDK compiler, LLVM, thin LTO and one codegen unit. It sets `AS` to the compiler
driver and uses the inventoried LLVM 22 libclang path. It does not copy Nix-linked
binaries into `/app`. The probe's Platform-runtime CLI checks pass on x86_64.
Graphics drivers, GUI, portals, production app installation and the final release
manifest still require actual proof.

For that long compile, use a transient user service from the project root after
the Nix gate has exited:

```sh
systemd-run --user --unit=mailstronaut-p01-flatpak-compile --collect --service-type=exec \
  --property="WorkingDirectory=$PWD" \
  --setenv=PATH=/run/current-system/sw/bin:/usr/bin:/bin \
  /run/current-system/sw/bin/bash "$PWD/experiments/p01-flatpak/run-compile.sh"
```

Inspect any existing unit before starting another run. The driver shares the
native gate's exclusive build lock and records `target/p01-flatpak-compile.log`
and `target/p01-flatpak-compile.status`. Wait for an inactive unit and a numeric
status; zero means compilation/installation into the probe app directory passed,
not that the application was run. A coding-UI restart leaves this service running;
a host reboot interrupts it. The staged SDK Cargo target remains in the project
and can be reused after interruption.

After a successful compile, check the installed probe binary in the Platform
runtime, with no SDK library path or executable search path:

```sh
bash experiments/p01-flatpak/isolated.sh bash experiments/p01-flatpak/runtime-smoke.sh
```

This exercises version and native Nix configuration evaluation without registering
a production application or starting the GUI.

References: [Flatpak builder options and sandbox](https://docs.flatpak.org/en/latest/flatpak-builder-command-reference.html),
[installation environment variables](https://docs.flatpak.org/en/latest/flatpak-command-reference.html#environment),
[LLVM extension usage](https://github.com/flathub/org.freedesktop.Sdk.Extension.llvm22#usage),
[Cargo vendor implementation](https://doc.rust-lang.org/stable/nightly-rustc/src/cargo/ops/cargo_vendor.rs.html).
