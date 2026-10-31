# P01 — passive resource observations and retained viewport

2026-10-31. This is a renderer/MIME component increment. **P01 and the application
remain incomplete.** Cached mail still opens through the native text reader;
this change does not enable the original-message HTML view.

## Resource policy and independent observations

The Ammonia policy now removes whole active subtrees, including forms, frames,
objects, media, SVG/MathML and scripts, as well as event attributes. Safe table
content, colspan and ordinary styles survive. Blitz still has an explicit no-I/O
NetProvider and inert navigation/shell providers. No script runtime, network
provider implementation, CSP assumption or toolkit fork was introduced.

The reproducible [observer](../../experiments/p01-blitz/privacy.sh) runs a prebuilt
current renderer test executable in `nix develop .#renderer-probe`. Bubblewrap
isolates its network/PID namespaces, replaces its resolver configuration and hides
host runtime sockets; its only additional capability binds the namespaced DNS
port. The root filesystem is read-only except the private project fixture. Test
files are synthetic and the user's mail, keyring and DNS service are not used.
This containment is **not** the production renderer's security boundary.

Positive control runs before denial: libc actually resolves a synthetic name
against the loopback NXDOMAIN fixture, a TCP endpoint returns a redirect response,
and four synthetic PNG/CSS/font/HTML files are read. Independent server receipts
and `strace -ff` observe DNS/UDP, HTTP/TCP and the exact readable paths. The same
tracer then follows the renderer and its threads through twelve hostile resource
variants. It records no socket/network operations and no accesses to those
email-specified paths. Even failed socket attempts would fail the assertion; the
namespace cannot turn an attempted load into a passing observation.

The matrix includes IP/hostname loopback, a private address, HTTPS/DNS, redirect
endpoint, file, valid data PNG, data HTML/SVG, blob, FTP and retired private URLs.
Markup covers image/srcset, CSS import/background/mask/cursor/list/font, prefetch,
preconnect, frames/srcdoc, form/file controls, objects, SVG, media, scripts, event
handlers and inert links. An exact host-supplied local raster paints in every
variant; 48 denial callbacks are observed. Script execution has no optional
permissive control because this renderer supplies no JavaScript runtime. Parsed
subtree/attribute removal is tested separately; network/file observers are real
positive controls, not proof of arbitrary code-execution safety.

Final observer run: `target/blitz-privacy.GqHDUj`, exit zero, using the retained
document implementation. Raw syscall logs and the synthetic fixtures remain in
that ignored private directory. The compact observer receipt is committed below.
This is a bounded call-path/fixture result, not fuzzing, a memory-safety audit or
native cached-message/portal acceptance.

## Retained viewport renderer

`Document::new` owns a fresh sanitized DOM, immutable raster table and CPU renderer.
`Document::paint(View)` reuses them across scale, color-scheme, size and CSS scroll
changes. Geometry is validated before viewport allocation; dimensions remain at
most 4096 per axis and eight million pixels. Scale must be finite in 0.5–8; scroll
must be finite/nonnegative and clamps to content. Excessive/nonfinite layout
extents are rejected at one million CSS pixels per axis. These are API/resource
bounds, not measured process-memory or CPU-time caps.

Real pixel tests exercise top/bottom scrolling, 2x physical scale with unchanged
logical width, a logical-width media-query resize, dark color-scheme media queries,
end-of-document clamping and recovery after invalid geometry. A 600-pixel-tall
document produces an 80-pixel-tall frame; raster length remains viewport-sized.
Oversized CSS height is rejected before painting. The existing table, PNG/JPEG,
alpha, CID/retirement and theme tests still pass. The one-frame convenience APIs
use this implementation, so the native synthetic preview continues exercising it.

This is not native resize/scroll/HiDPI or latency/memory acceptance. Connecting
actual native bounds/scrollbars, generation/lifetime handling and MIME images is
the next frontend step. Native text remains the accessibility/selection route;
no accessibility tree is claimed for a raster.

## MIME resource extraction

`Projection::inline_rasters` reuses mail-parser with the existing part/depth/nested
message limits. It verifies the original SHA-256, descriptor ranges and exact
message-local identity, checks ambiguity/truncation and transfer-decode problems,
and retains only PNG/JPEG compressed bytes under 4 MiB per part/16 MiB aggregate.
Declared MIME type is case-insensitive. SVG/other attachments are not decoded for
HTML. Encryption suppresses extraction.

MIME Content-IDs stay opaque data; even a path-like ID is never read as a path.
The renderer applies its stricter 255-byte key alphabet and ambiguity policy before
installing pixels. It separately checks raster signatures, dimensions and decoded
pixel budgets. Extraction does not claim to validate a PNG from its MIME label.
Tests exercise library base64 decoding, mixed-case types, filename independence,
opaque IDs, ambiguity, bad/unknown encoding, changed original, truncation and
oversized parts. No MIME decoder or image codec was reimplemented.

## Local validation

Pinned development formatting, tool lint, strict workspace Clippy and 141 normal
tests passed. Targeted final MIME checks passed after the case-insensitive MIME
adjustment. The renderer has 13 passing normal tests plus the explicitly ignored
observer invocation; MIME has 14 passing tests. No dependency/lockfile, vendor hash,
compiler-backend exception or application runtime input changed.

The current-source local Nix gate passed on x86_64 with all 16 available CPUs and
one Nix build job, stable LLVM, thin LTO and one codegen unit. Strict release
Clippy, 141 normal tests and five private credential/OAuth/account-action checks
passed. The ignored resource observer is exercised separately above. The service
`mailstronaut-passive-viewport-gate.service`, invocation
`cf1fab19b0a14337bbc1676195623b85`, finished with a fresh numeric receipt of zero,
all checks passed in its full log and its collected service was inactive with no
MainPID. Aarch64 was omitted.

The cached `nix build` produced
`/nix/store/nkl4paqc6qhjsjr44vy0k6xmzz5kqwc0-mailstronaut-0.1.0`.
All 129 files in its filtered source
`/nix/store/fm1hx4girvz9x1nnx4yfxswmvr328j2w-source` match the checkout byte-for-byte.
The installed empty-environment CLI/native-config smoke and native Wayland
regression passed against this exact package. The latter exercised the Blitz
preview/local-raster colors, light/dark, long read-only Unicode text, selection/
copy/navigation and X11-only startup rejection in `target/gpui-wayland.2btQts`.
This is the current synthetic preview, not a newly enabled cached HTML reader.

- [Source/gate identity and hashes](artifacts/p01-passive-viewport/source-and-gate.json)
- [Resource observer receipt](artifacts/p01-passive-viewport/observer-receipt.json)
- [Positive-control syscall excerpt](artifacts/p01-passive-viewport/control-trace-excerpt.log)
- [Complete compact observer output](artifacts/p01-passive-viewport/observer.log)
- [Release gate excerpt](artifacts/p01-passive-viewport/release-excerpt.log)
- [Installed CLI smoke](artifacts/p01-passive-viewport/installed-cli.log)
- [Installed Wayland regression](artifacts/p01-passive-viewport/installed-wayland.log)
- [Current native synthetic preview](artifacts/p01-passive-viewport/dark.png)
- [Native text reader](artifacts/p01-passive-viewport/reader-end.png)

The full local logs remain under `target/blitz-reader-*`, `target/blitz-privacy-*`
and `target/p01-package-gate-service.log`. Hosted CI is not a prerequisite for
continuing development after these local Nix checks.

## Remaining work

Connect a single bounded renderer worker to native viewport/scroll changes and
the selected original, prove stale-response/resource retirement through actual
message switching/hiding and then enable cached HTML. Complete ordinary malformed/
nested layout corpus, explicit portal links, selection/accessibility, repeated
switch/hidden-idle/peak-memory measurements and Flatpak acceptance. The complete
product goal remains active; composition, background reminders, strong semantic
search and local priority/deadline classification remain elsewhere in PLAN.
