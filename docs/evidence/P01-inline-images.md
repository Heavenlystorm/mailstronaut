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
workspace entries. No upstream package version or source changes. The standard
Nix vendor build passed with the existing hash; the changed lockfile only adds
those two workspace dependency references.

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
