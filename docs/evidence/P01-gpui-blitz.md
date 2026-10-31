# P01 — GPUI Kit + Blitz foundation

Later [cached HTML integration](P01-cached-html.md) supersedes the initial
text-only cached-mail scope below. This document records the original foundation
at its stated revision; PLAN.md owns current progress.

2026-10-30. This record replaces frontend acceptance claims for the retired
stack; P01 remains **open**. The source/lockfile and local commands in this change
are authoritative. No live mail/account was used in these fixtures.

## Implemented foundation

- GPUI Kit 0.8.1 selects GPUI pre 0.3.10. Standard TitleBar, Button, Input,
  resizable panels, virtualized list and semantic themes provide the native UI.
  Startup allows Wayland only, clears the X11 display in the owned windowing
  request and rejects a missing Wayland display. The published facade still
  compiles its X11 feature; the application does not connect to X11/XWayland.
- Blitz DOM/HTML/paint/traits 0.3.0-beta.2, AnyRender 0.13 and Vello CPU 0.17
  paint actual HTML/CSS into owned pixels. RGBA is converted to GPUI's BGRA.
  Layout/raster runs on a blocking task; stale generation results are discarded.
  There is no foreign browser surface or custom GPU interoperability.
- Ammonia removes active markup/attributes. An explicit no-I/O resource provider
  denies every supplied request; navigation and shell providers are explicitly
  inert. No JS runtime, CSS fetching, SVG mail renderer or file-input feature is
  enabled. Input is limited to 256 KiB, sanitized output to twice that, DOM to
  4096 nodes/128 depth, sides to 4096 and frames to 8 million pixels.
- The real cached inbox and literal local search connect through the existing
  store owner. Successful explicit opens load the cached original into a native
  text view and queue only that original's Seen intent with the validated account
  context. The previous explicit-open authorization gate and two behavior tests
  are retained. Native config/helper CLI commands remain display independent.
- `--demo` uses only synthetic rows and fixed HTML; it never opens the account
  store. Only this controlled fixture currently enters Blitz in the application.
  Cached untrusted HTML stays on the MIME library's inert text conversion path.

## Development checks

Inside `nix develop`:

```
bash ci/check.sh
cargo build --locked --offline
```

Passed format/Nix format, actionlint, ShellCheck, strict all-target Clippy and
**132 workspace tests**; seven private/live fixtures were ignored in this normal
run. The six renderer tests exercise real red/green table cells and Unicode,
active-markup stripping, invalid raster/input/DOM budgets, native CSS light/dark
media queries and denied image/CSS requests. A real loopback listener first
observes an explicit positive-control connection, then observes no connection
from the denied HTML fixtures. This is bounded loopback coverage, not a complete
DNS/filesystem/redirect/resource security proof.

Cranelift required narrowly scoped LLVM debug/test exceptions: Pulp's exact
assembly link names and fearless_simd/Vello Common/CPU's AVX2 kernels. A
fearless-only exception still aborted in `vello_common::strip::render_impl`.
Other packages use Cranelift; debug/test linking uses Wild. Release remains
stable LLVM, thin LTO and one codegen unit. No dependency forks were introduced.
GPUI selects oo7 async-std; the old simultaneous Tokio choice failed ashpd
feature selection. The unified oo7 backend now matches GPUI. Private encrypted
credential fixtures must be revalidated in the release gate.

## Native Wayland probe

```
nix develop .#gui-test --command bash experiments/p01-gpui/wayland.sh target/debug/mailstronaut
```

Passed on x86_64-linux, with private XDG roots, an activation-free private D-Bus,
headless Sway (`xwayland disable`), pinned Mesa software rendering, Grim and
Wtype. `DISPLAY` is unset. An X11-only startup is separately rejected.
The Wayland protocol identifies the application's xdg_toplevel and commits.
Actual screenshots show the corrected virtual list and real Blitz content.
Ctrl+K plus typed synthetic text and Ctrl+Shift+D work in the real window;
explicit pixel checks verify dark chrome **and** dark HTML, not just command exit.
The light and dark screenshots were also visually inspected.

- [Light window](artifacts/p01-gpui-blitz/native-light.png)
- [Dark window with keyboard search input](artifacts/p01-gpui-blitz/native-dark.png)
- [Protocol excerpt](artifacts/p01-gpui-blitz/wayland-excerpt.log)

The initial Weston headless probe lacked a seat required by GPUI. Standard Sway
provides it; no custom compositor or protocol implementation was added. Wlrctl's
short-lived keyboard vanished before GPUI bound it, so the final probe uses
Wtype with a short device-binding delay. The test now checks resulting pixels.
These are isolated native Wayland results, not hardware-GPU, desktop portals,
packaged-window, screen-reader, resize/HiDPI or performance acceptance.

## Remaining acceptance

The local Nix release/package, installed CLI and installed Wayland checks passed
as recorded below. The complete x86_64 [hosted CI gate](P01-ci-cache.md) later
passed at `c4d34a1`, including installed CLI/config smoke. Aarch64, Flatpak and
exact minimum-Rust acceptance remain separate gates.
The previous GitHub run `41072214708` failed from build-disk exhaustion in the
retired dependency graph; it does not validate or invalidate this replacement.

P01 still needs the full resource/privacy matrix, message-local CID/raster
resources, ordinary/nested/malformed real-world synthetic email corpus, responsive
viewport/HiDPI and cancellation/memory measurements, links, selection/copy and
actual accessibility. Blitz's published table/border limitations remain visible;
no Aviary patches were imported. Native accounts/sync/compose/reminders need GPUI
workflows even though their backend persists. Strong fuzzy/multilingual meaning
search and local priority/deadline inference are not implemented by this slice.

## Stable Nix package acceptance

Implementation commit: `ae67c6526a8f89a30ec4928e1bac8c330deb6b44`.
The coordinated service invocation `fa8e90a0b9d3465c9f37c84993758734` ran
`ci/nix-check.sh` with 16 Nix cores and one Nix job. Its fresh numeric receipt was
`0`; the service was collected/inactive, with `ExecMainStatus=0`, and the complete
log ended `all checks passed`. No second release build ran in parallel.

The gate validated the new fixed vendor hash, Nix/CI formatting, stable Rust/Cargo
1.99.0, strict release Clippy, **132 release workspace tests** and four private
Secret Service/OAuth fixture invocations (three distinct cases, with the
missing-service case repeated). The encrypted adapter therefore passes the
private native fixtures after the oo7 backend feature change; Flatpak portal
credentials and live-provider sign-in remain unverified. Release is LLVM, thin
LTO, one codegen unit; development backend exceptions are excluded.

The subsequent `nix build --no-update-lock-file` and installed version/config
init/config checks passed inside `nix develop`. Installed output:
`/nix/store/gfb67bx585d5vj7w0h79q5dlwz4an0y6-mailstronaut-0.1.0`.
The same native Wayland script passed against `result/bin/mailstronaut`, including
real keyboard input, dark chrome/HTML pixel checks and rejection of X11-only
startup. The installed dark screenshot was visually inspected. These package
results cover x86_64-linux only; Nix explicitly omitted aarch64-linux.

- [Release gate excerpt](artifacts/p01-gpui-blitz/release-gate-excerpt.log)
- [Installed CLI/config output](artifacts/p01-gpui-blitz/installed-smoke.log)
- [Installed Wayland dark window](artifacts/p01-gpui-blitz/installed-dark.png)

Full logs and disposable private fixtures remain under project `target/`.
This is local package/native-window acceptance, not full P01 privacy,
accessibility, performance, hosted CI or Flatpak acceptance.

The subsequent hosted build failed from disk exhaustion. See the separate
[CI cache repair record](P01-ci-cache.md) for the immutable-vendor correction,
local source-only reuse checks and their sandbox/hosted acceptance boundaries.
