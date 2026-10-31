# GPUI Kit + Blitz rendering

This is the current rendering contract. The earlier browser/toolkit experiments
are historical, not an alternative implementation or acceptance of this stack.
See PLAN P01/P06 and `evidence/P01-gpui-blitz.md` for actual progress.

## Native embedding

Use GPUI Kit's published facade, matching GPUI family, standard styled components,
assets, semantic themes, focus and virtualized lists. The facade pins its framework
family, avoiding independently selected incompatible GPUI types. Application code
uses its reexports. Do not enable the optional JavaScript extension runtime.

Native Wayland is mandatory. The published facade compiles Wayland and optional
X11 backend support together; Mailstronaut must select Wayland and never fall back
to an X server. Prove a real window with DISPLAY unset and no XWayland. Runtime
libraries compiled into a distribution are distinct from requiring an X server.
The HTML route uses no separate native window or platform webview.

Use Blitz's `HtmlDocument`/`DocumentConfig`, public viewport/layout APIs and
`blitz_paint::paint_scene`. Render with the matching AnyRender Vello CPU
`ImageRenderer` and supply the owned raster through GPUI's normal image API.
Aviary demonstrates this overall approach; its implementation, optimizations and
vendored patches are not architectural requirements. Begin without those forks.

One bounded worker owns the active document/layout/raster context. Messages to it
carry owned HTML/resources and generation/size/theme requests. Replies contain
owned bounded pixels and metadata. Retire superseded work and ignore stale replies;
never transfer thread-affine DOM/font objects or native GPU resources between UI
and renderer. Render on changes, not on a permanent polling/redraw loop.

## Untrusted content boundary

Use ammonia's HTML parser/allowlist, preserving ordinary tables, text, inline CSS
and safe style rules where supported. Strip scripts, event attributes, frames,
forms/inputs, objects/embeds, base/meta refresh, executable URLs and active content.
Do not sanitize HTML or CSS with regex. A style allowlist alone is not network
policy, and Blitz must not be assumed to enforce browser CSP directives.

Blitz core is modular: omit its networking/window wrappers and any JS runtime.
Install an explicit default-deny `NetProvider`, inert `NavigationProvider` and
inert `ShellProvider`. Those providers grant no filesystem/network, clipboard,
file-dialog, popup, titlebar/native-command or credential authority to mail.
Remote images, styles/imports, fonts, redirects, localhost/private IPs, `file:`,
`data:` HTML/SVG and unknown schemes are denied by default. No ambient cookie or
HTTP cache is attached. System font discovery is an explicit platform input,
not permission for email-specified file paths.

Each message has a fresh, immutable resource table owned by its document. Resolve
CID descriptors with the MIME library; never interpret a CID/filename as a path.
Only exact known message-local raster resources can be supplied. Reject duplicate
or ambiguous IDs, encoded/traversal variants, missing/retired resources and wrong
content types. Decode dimensions and encoded/decoded bytes must be bounded before
supplying an image to the engine. Active SVG and unbounded embedded data remain
disabled until separately supported and tested.

The retained renderer entry point is `Document::new` followed by `Document::paint`;
`render` and `render_with_images` remain one-frame convenience functions. The host supplies
borrowed, transfer-decoded PNG/JPEG bytes and MIME Content-IDs. Each call validates
at most 64 images, 4 MiB encoded per image, and 16 MiB aggregate encoded plus
retained RGBA bytes. Dimensions are inspected before pixel decode with the same
strict 4096-side limit used during decoding. The image library's decoder allocation
limit is also set for header inspection and decoding, but is best-effort; this is
not a measured process-memory cap, and scratch/font/frame memory is additional.

Ammonia rewrites only exact known `<img src="cid:...">` values into a fresh private
URL namespace. Unknown, encoded, path-like and external image sources are removed.
IDs are case-sensitive conservative ASCII keys up to 255 bytes, never filenames.
The host installs already decoded RGBA with Blitz's public `load_resource` API;
encoded bytes never reach Blitz's own image decoder. Every `NetProvider::fetch`
still drops its handler without I/O, even for supplied images; the independent
cache supplies those pixels. No process-global image registry is retained.

This path is wired into the synthetic Wayland preview and cached HTML reader.
`Projection::inline_rasters` reparses the bounded, revision-checked original with mail-parser and obtains
transfer-decoded PNG/JPEG bytes. It rejects ambiguous/truncated parts, decode
problems and oversized resources; MIME IDs remain opaque data, with the stricter
CID-key/signature/dimension checks applied in the renderer. The owner worker
prepares these resources off the UI thread. Invalid extraction or image validation
omits the local images with a visible notice; otherwise unsupported HTML falls
back to the already loaded native text alternative. CSS CID backgrounds, SVG, GIF
and arbitrary data URLs are not enabled. A private namespace is an application
lookup key, not a claim about browser-origin semantics.

Automatic navigation is inert. A future explicit GPUI link action may invoke the
standard external-browser portal after checking the current document and allowed
URL scheme. There is no login page inside the reader. Forms and email content do
not call native actions, even when a resource is locally available.

## Budgets, layout and lifetime

Initial implementation limits: decoded HTML at most 256 KiB, bounded node/depth
counts, at most 16 MiB supplied local resources, frame dimensions at most 4096 per
axis and at most 8 million pixels. Validate finite scale/scroll/size values before
allocation or narrowing integer conversions. Tune limits from recorded fixtures,
not from an unbounded document. Rasterize the viewport rather than an entire long
newsletter; reuse the active document and avoid painting unrelated offscreen work.

`Document` now retains that DOM, font context and Vello CPU raster context across
`paint(View)` calls. `View` uses physical width/height, native scale in 0.5–8 and
nonnegative finite CSS scroll coordinates. Scale/media changes go through Blitz's
public viewport API; scroll clamps to the current content extent. CSS extent is
limited to one million pixels per axis before painting. `Frame` reports that
extent for future native scrollbars but contains only viewport-sized RGBA bytes.
The GUI has one document-owner thread, a latest-value request channel and a
one-frame reply channel per window. Native geometry, scroll, scale and theme
changes repaint only the viewport; scroll alone does not relayout the immutable
DOM. Superseded requests coalesce, obsolete replies are ignored, and source changes
or hiding retire the prior DOM/resource table. BGRA conversion runs on that same
owner thread. GPUI's normal image paint API presents the result; there is no
custom GPU interop or forwarded HTML input event.

GPUI Kit's segmented `TabBar` and `Toolbar` select HTML or native text. Standard
`ScrollHandle` and Kit horizontal/vertical scrollbars own scrolling; a small GPUI
canvas bridge measures their public bounds and paints the matching viewport.
Home/End/Page Up/Page Down act on the scroll handle. Invalid native geometry
falls back to text. Successfully presenting a current frame can consume an
existing explicit-open authorization for Seen; layout completion, passive focus,
theme changes and page restoration cannot create one.

The independent privacy probe is `experiments/p01-blitz/privacy.sh` in the pinned
`renderer-probe` shell. It uses an isolated network namespace and observes actual
file/network syscalls with strace. Actual DNS resolution, TCP redirect response
and four readable-file controls precede the denied fixture matrix. The namespace
prevents a test regression from contacting host services; it does not hide socket
attempts from the observer or supply a production sandbox. See the exact tested
scope in [P01 passive/viewport evidence](evidence/P01-passive-viewport.md).

Start with ordinary block/table/inline email layouts, inline CSS and system fonts.
Exercise nested tables, colspan/rowspan, collapsed borders, fixed/relative widths,
long unbroken content, malformed markup, Unicode and missing images. Blitz has
incomplete browser compatibility: document actual limitations and provide a native
text reader instead of silently substituting another engine or stripping all layout.

Use Blitz's viewport color scheme/media handling. A GPUI Kit dark/light theme does
not imply that a hard-coded white email becomes dark; do not invert raster images.
Keep native controls/theme and document colors distinct. Resize/HiDPI changes
invalidate frames; generations prevent an old width/theme/message from repainting.
Hide/close cancels rendering and releases message data when no longer needed.

The native text reader uses GPUI Kit's `TextareaState`/`Textarea::readonly(true)`
with a bounded MIME text projection, an accessible label and a focus action.
It accepts no edits; selection/copy, wrapping and keyboard scrolling remain the
component's responsibility. Focus and appearance changes never reopen mail or
authorize Seen. The demo includes a synthetic long text alternative alongside
the actual Blitz preview. The Blitz image alone is not an accessibility tree.
Verify Unicode copy,
keyboard navigation/scrolling, focus and actual Wayland/AT-SPI exposure. HTML hit
handling/selection and a possible accessibility subtree bridge need explicit
public-API, lifecycle and user-session acceptance before claiming support.

## Required proof

- Real GPUI Kit Wayland window and Blitz pixels, with DISPLAY unset/no XWayland.
- Synthetic text/table/layout/image fixtures and independent pixel observations;
  test RGBA/BGRA ordering, alpha, physical scale, clipping and stale generations.
- Denied resource/script/form/frame/file/CSS/font/SVG/navigation cases with working
  observer controls. No fetch is not proof if the observer could not see a fetch.
- Message-local raster positive control, CID ambiguity/retirement/isolation and
  oversized/decode-bomb limits. No real mail or user files in fixtures.
- Actual key/focus/copy behavior and accessible text evidence; distinguish headless
  component tests from a real Wayland window and assistive-technology session.
- Long-email layout/scroll response, repeated switches, bounded memory and hidden
  idle CPU; clean exit/drop and cancellation under resize/navigation stress.
- Nix package, installed CLI/config, actual installed Wayland window and Flatpak
  SDK/runtime evidence recorded separately. Source inspection does not close them.

## Selected API sources

- [GPUI Kit facade](https://docs.rs/gpui-kit/0.8.1/gpui_kit/) and
  [getting started](https://gpui-kit.com/docs/getting-started/).
- [Blitz source](https://github.com/DioxusLabs/blitz) and the published 0.3.0-beta.2
  DOM/HTML/paint/traits crates. Keep the four versions aligned.
- [Aviary reference](https://github.com/Catvert/aviary): CPU raster/image embedding;
  inspect exact revisions and patches before drawing compatibility conclusions.
