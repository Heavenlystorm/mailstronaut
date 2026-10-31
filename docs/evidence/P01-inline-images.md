# P01 — bounded inline PNG/JPEG resources

2026-10-31. P01 remains open. This implements the renderer's resource table and
synthetic preview, not the original-message MIME connector or full HTML privacy
acceptance. Real cached messages still use the native text reader.

## Implementation

`render_with_images` accepts borrowed transfer-decoded PNG/JPEG bytes, declared
MIME types and opaque Content-IDs. The existing `image` library checks signatures,
dimensions and decodes pixels. Both header inspection and decoding use strict
dimension limits plus the library's best-effort allocation limit. Before pixel
decode, the host enforces 64 entries, 4 MiB encoded per image, 4096 per side and
16 MiB aggregate encoded plus retained RGBA bytes. Decoder scratch, font/DOM/frame
memory and process peak memory are separate; no hard process-memory cap is claimed.

Only known `img src="cid:..."` sources are rewritten by Ammonia's parsed attribute
filter to a fresh private URL namespace. Case-sensitive ASCII keys are at most
255 bytes; duplicates, percent encoding, URL delimiters and path separators are
rejected. Unknown or external image sources are removed. No CID becomes a file
path. CSS CID backgrounds, GIF, SVG and data images remain unsupported.

The application supplies decoded RGBA through Blitz's existing public
`BaseDocument::load_resource`/`Resource::Image` cache API. Encoded attachment data
never reaches Blitz's unrestricted image decoder. The network provider never
delivers bytes: every fetch still drops its handler without I/O. Supplied image
URLs are excluded from the blocked-resource count because the host fills their
cache independently. Each call owns a new DOM/table/CPU raster context and drops
its image references before returning an independent frame. This is document-local
lookup isolation, not a browser-origin claim.

The only new direct dependencies are the already locked `image` and `uuid`
workspace entries. No upstream package version or source changes. The lockfile
adds those two workspace references, but the standard Nix fixed-output staging
also stores the lockfile itself. Its hash therefore must change. An initially
cached vendor output hid this; a forced fresh staging rebuild correctly rejected
the old hash and reported `sha256-ZGIoa0S/uiLjnuuWVYiCeWV59VErlAk/QcHd88ReBLQ=`.
The flake now declares that measured hash. Fresh-probe log:
`target/gpui-inline-images-fresh-vendor.log`.

## Development and rendering evidence

Through `nix develop`:

```
cargo fmt --all
bash ci/check.sh
cargo build --workspace --all-features --locked --offline
```

Passed formatting, shell/CI lint, strict workspace Clippy and **136 normal tests**,
including ten renderer tests. Seven private/live cases remain ignored in the
normal suite. The focused renderer tests establish:

- Actual PNG and JPEG pixels appear in Blitz's raster; transparent PNG composes
  to the expected RGBA color over white.
- A renders its image, B with no matching resource cannot read A's CID, and a
  later fresh A paints correctly. Distinct blue B markers prevent stale A frames
  from satisfying the foreign/missing-source checks.
- Unknown, encoded, traversal, foreign namespace, file, data and remote `img`
  sources remain absent. Duplicate/malformed IDs, invalid image data and MIME
  mismatches fail with redacted errors.
- Oversized encoded data, dimensions, decoded expansion, aggregate table bytes
  and image count fail. A small compressed PNG cannot bypass the decoded budget.
- Existing CSS/network denial fixtures still pass; their loopback observer has
  a working connection control. Image URLs are now stripped before those requests
  could reach Blitz.

This is bounded synthetic coverage. It is not independent DNS/file tracing, a
complete active-content matrix, a decode-bomb fuzz corpus, or memory/latency proof.

## Native Wayland preview

```
nix develop .#gui-test --command bash experiments/p01-gpui/wayland.sh target/debug/mailstronaut
```

Passed in the existing private Sway/XDG/D-Bus profile with DISPLAY unset and
XWayland disabled. The demo supplies a versioned 24×8 RGB PNG as a local resource.
Independent screenshot masks observe red, green and blue bands in the correct
horizontal order and at the same height in both light and dark modes. This also
checks the Vello RGBA → GPUI BGRA conversion and that theme changes do not invert
the image. Existing search/theme, X11-only rejection and read-only Unicode
selection/copy/navigation checks passed. The dark screenshot was visually inspected.

- [Dark inline-image preview](artifacts/p01-inline-images/native-dark.png)
- [Observed color geometry](artifacts/p01-inline-images/inline-colors.log)

Private profile: `target/gpui-wayland.KhvzLq`. Full development and preview logs:
`target/gpui-inline-images-workspace.log`, `target/gpui-inline-images-wayland.log`.
All content is synthetic; no account or personal mail was used.

## Narrow development backend exception

The first actual image painting test aborted on unsupported
`llvm.x86.avx2.psrl.w`. Its backtrace identifies
`anyrender_vello_cpu::image_convert::premultiply_rgba8_impl`, which instantiates
the inline fearless_simd kernel. The existing four dependency overrides did not
cover this call site. LLVM for `anyrender_vello_cpu` alone fixes the demonstrated
failure; the ten renderer tests and full workspace checks then passed. Other
development packages retain their selected backend. Release remains stable LLVM,
thin LTO and one codegen unit. No dependency fork or scalar renderer was added.

- [Cranelift diagnostic excerpt](artifacts/p01-inline-images/cranelift-excerpt.log)

The full diagnostic is `target/gpui-inline-images-cranelift.log`. Remove the scoped
debug/test exception when the pinned Cranelift supports that SIMD operation.

## Remaining work

Original-message MIME resource extraction, asynchronous retirement/cancellation,
cached HTML enablement, the full resource/privacy matrix, viewport/HiDPI, links,
HTML selection and accessibility remain separate. Nix release/package, installed
Wayland, hosted CI and Flatpak acceptance for this change are recorded only when
their own checks finish.

## Initial cached Nix and installed checks

Implementation commit: `0a0fdc53d0b40cd1b7a3079a78b1d7562514ffbf`.
Before the fresh staging check above, the single coordinated release gate, invocation
`e25bc925800a4a87b23a084aabef7116`, completed with a fresh receipt `0`, full
`all checks passed` log, inactive/collected service and `ExecMainStatus=0`.
It used 16 cores and one Nix job. Strict stable release Clippy, **136 normal
tests** and four private credential/OAuth fixture invocations passed. Only the
five workspace crates compiled/checked in the real package after restoring the
new dependency artifact. Aarch64 was omitted.

- Package: `/nix/store/c5bv1kxiwpbi7wj9zmvyr505hpl131kw-mailstronaut-0.1.0`
- Dependency artifact: `/nix/store/adf976i645f6jydpaizl8yw6gp0mim5r-mailstronaut-deps-0.1.0`

The subsequent cached `nix build --no-update-lock-file` and installed empty-env
CLI/configuration smoke passed. The complete private Wayland script also passed
against `result/bin/mailstronaut`, including ordered RGB image bands in both
themes, native reader copy/edit protection/navigation and X11-only rejection.
The installed dark screenshot was visually inspected. Private fixtures:
`target/gpui-wayland.3d2eb1`, `target/p01-installed-smoke.6UqAse`.

- [Installed dark preview](artifacts/p01-inline-images/installed-dark.png)
- [Installed pixel observations](artifacts/p01-inline-images/installed-colors.log)
- [Installed smoke output](artifacts/p01-inline-images/installed-smoke.log)

The full release log is `target/p01-package-gate-service.log`; installed output
is `target/gpui-inline-images-installed.log`. Hosted CI for this commit and
Flatpak remain unverified. These cached checks alone do not validate the new
fixed-output input; the corrected-hash gate is recorded separately below.
The resource/MIME/privacy/lifecycle limitations above
remain open; this is not whole-P01 or functional-client acceptance.

## Corrected-input release and resumed verification

Vendor correction: `e875ede469cb26fb274eabef2262cfcf4e135d70`.
Fresh staging accepted the measured hash recorded above. The coordinated gate
invocation `72ef35918d454521bdd7e3fe274f4f06` finished with receipt `0`, a full
`all checks passed` log and 136 normal release tests plus four private
credential/OAuth invocations. After resuming, the unit was confirmed inactive,
collected, with `ExecMainStatus=0`; its journal records the successful exit.

- Corrected package: `/nix/store/hg1zndb04s9j7lrmcq3ygs4z67gdjqf8-mailstronaut-0.1.0`
- Dependency artifact: `/nix/store/xbdj95vzzsrj5y5s5p1yrj5xxmkm1igq-mailstronaut-deps-0.1.0`

The corrected package passed the installed empty-environment CLI/config smoke.
Its full private Wayland probe then passed through `nix develop .#gui-test`,
including the ordered RGB bands in light/dark modes, native reader interactions
and rejection of an X11-only session. Profile: `target/gpui-wayland.87E8mz`.
The installed screenshot linked above now comes from this corrected package.
Nix reported an ignored evaluation-cache contention warning during concurrent
checks; both commands exited successfully and the application probe passed.

The pre-compilation vendor-lock guard compares raw hashes using Nix itself,
without requiring additional tools in the minimal container. Executing its exact
comparison block against the fresh vendor succeeds; the previously cached stale
vendor fails with the intended diagnostic. The resumed pinned `ci/check.sh`
(136 normal tests, strict Clippy and format/shell/CI lint) and full
`ci/nix-check.sh` both passed with this guard. Aarch64 remains omitted.

Logs: `target/gpui-vendor-lock-guard.log`,
`target/gpui-inline-images-resume-checks.log`,
`target/gpui-inline-images-final-wayland.log` and
`target/gpui-inline-images-final-installed-cli.log`.
Hosted CI, MIME/HTML enablement, lifecycle/privacy and Flatpak remain separate.
