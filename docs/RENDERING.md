# Servo integration gate and email rendering policy

The first feature work is a focused integration experiment, before account/sync UI.
Servo is the requested engine. Its existence is not proof that it fits libcosmic's
renderer/event loop, safe email resource policy or accessibility requirements.

## 1. What is known

- Current inspected libcosmic commit `8990a...` has Iced as a submodule at
  `pop-os/iced` commit `229186b5180b6217d93866c912e6373ac378b231`, not an arbitrary
  crates.io Iced. Use `cosmic::iced` types.
- Servo release/API documentation is 0.7.0. It exposes ServoBuilder,
  WebViewBuilder, RenderingContext, EventLoopWaker, WebView::paint and a theme
  notification method. These are an embedding foundation, not a COSMIC widget.
- `iced_webview_v2` main at `3ed0ad...` depends on Iced 0.14 and Servo 0.1. Its
  README's next branch tracks still other versions, and some README claims lag
  the manifest. It renders Servo into a CPU pixel buffer and uploads a texture.
  The inspected adapter does not expose the complete theme/resource security
  policy required here. Do not paste its example's perpetual 10 ms timer into
  a mail client and call idle behavior finished.

Sources: [libcosmic manifest](https://github.com/pop-os/libcosmic/blob/8990a058e1deb8f9f7f4c0a42f6b1362fa1437ec/Cargo.toml),
[wrapper manifest](https://github.com/franzos/iced_webview_v2/blob/3ed0ad922e130faf2f5be3a0ff602bfa292a0272/Cargo.toml),
[wrapper Servo code](https://github.com/franzos/iced_webview_v2/blob/3ed0ad922e130faf2f5be3a0ff602bfa292a0272/src/engines/servo.rs),
[Servo embedding](https://book.servo.org/embedding/overview.html).

## 2. Exact experiment sequence

1. Pin the libcosmic commit with submodules, selected Servo release commit and the
   toolchain in Nix/Cargo.lock. Record native dependencies and network-free build
   behavior. Run `cargo tree -d` and inspect all Iced/Stylo/wgpu/links dependencies.
2. Build a minimal libcosmic window using normal cosmic Application/Core/widgets,
   with navigation and a text pane. Confirm Wayland and fallback X11, live system
   theme and accessibility. Do not build the full inbox yet.
3. Attempt to reuse the existing Iced/Servo adapter. Align **every relevant Iced
   subcrate** with libcosmic's source through supported upstream pins/patches;
   demonstrate that `Element`, renderer and shader types actually compose. An
   `iced` semver match alone does not prove source/type compatibility.
4. If reuse would force a broad fork, use Servo's existing offscreen rendering API
   in a thin local adapter with the same responsibilities. Start with a bounded
   viewport-sized pixel buffer uploaded via libcosmic's image/texture path. A full
   document-height screenshot is unacceptable for large mail. Zero-copy graphics
   interop is not required before measurements show a bottleneck.
5. Drive Servo through EventLoopWaker→UI wake/event, spin_event_loop, frame-ready
   delegate, WebView paint, context presentation/copy as appropriate to the pinned
   API. Coalesce wakes and repaint only on changed frames/viewport damage. Preserve
   GUI-thread affinity and process platform events normally; no second competing
   winit event loop or busy polling. If an upstream constraint requires bounded
   polling, measure/document it and stop that polling when hidden/idle.
6. Translate viewport resize/HiDPI/scroll/pointer/focus/keyboard events, copy/select,
   cursor and link activation through upstream APIs. Test Ctrl+C, selection,
   non-ASCII text, 100/150/200% scale, resizing and long-message scroll. There is one
   active message webview, not a Servo engine instance per inbox row.
7. Forward effective COSMIC theme to `WebView::notify_theme_change`. Switch light↔dark
   while an email is visible; verify `prefers-color-scheme` fixtures. A fixed white
   sender background may stay white. No CSS inversion or custom dark renderer.
8. Install resource/navigation/permission delegates and the policy below. Load
   malicious-resource fixtures under network observation and prove default-deny.
9. Close all windows, drop webviews/render resources and prove sync/reminder work
   can continue without a surface. Reopen repeatedly without GPU/thread leaks.
10. Record a decision in `docs/evidence/P01.md`: exact revision/features/patches,
    commands, screenshots, CPU/memory numbers, accessibility results and failures.
    Remove throwaway spike scaffolding after adopting the working adapter.

Stop gate: P01 is incomplete until actual composition, theme, resources, input,
Nix build and lifecycle are proven. A fake image pane, a separate browser window,
incompatible duplicate Iced versions, or turning off security is not success.
If any is blocked, document the precise missing upstream hook, a bounded fix, and
keep dependent HTML-release tasks unchecked. Do not silently change engines.

Pinned lifecycle constraint: Servo 0.7 calls `servo_config::opts::initialize_options`
on every engine construction, but that function sets a process-global `OnceLock`
and panics on a second call. Its SpiderMonkey dependency also rejects initialization
after engine shutdown. Dropping and reconstructing the entire engine in the same
application process is therefore not a supported lifecycle in this pin. The next
synthetic slice retains one engine and one reusable software context, drops and
recreates its WebView, and shrinks the idle context to 1×1. This deliberately
retains graphics resources while the GUI engine lives; complete release while
headless still needs a separate solution and measurement. Upstream already tests painter
removal on the last webview's drop (`servo` 0.7.0 `tests/webview.rs`); reuse that
behavior rather than inventing a renderer teardown mechanism. This does not meet
the architecture's complete browser-release requirement by itself. Resolve that
gate explicitly before P08; do not silently retain a hidden browser indefinitely
or introduce a helper-process architecture without recording its design and cost.

## 3. Sanitization and resource model

Use `ammonia::Builder` to parse/serialize untrusted HTML with an explicit email
allowlist. Preserve useful table/text/email layout; remove script/handlers,
iframes, object/embed, forms/inputs, base, meta refresh, active SVG and unsafe URL
schemes. Do not try to sanitize HTML with regex. Ammonia's default output strips
many styles; blindly using defaults fails the layout goal. Conversely, allowing
`style` attributes or `<style>` is **not** a CSS sanitizer. Validate compatibility
and resource safety before permitting them. Prefer engine enforcement to a custom
CSS rewrite library; do not remove all layout just to pass a screenshot.

Create a per-message opaque document origin and resource table. Top-level body
and CID attachments are memory-backed, immutable, with content-type/size limits.
CID resolution is scoped to the selected message and MIME part; duplicates,
percent encoding, missing IDs and traversal strings do not escape that scope.
Reject disk `file:` loads, localhost/private-network requests, data URLs outside a
small bounded raster-image allowlist, javascript:, arbitrary custom handlers and
navigation into another message's resource namespace. `data:` HTML/SVG is denied.

Use Servo's protocol registry for local resources and both
`WebViewDelegate::load_web_resource` and `ServoDelegate::load_web_resource` for
HTTP/HTTPS interception where applicable. An unhandled callback permits normal
networking in the documented API, so it must explicitly return a blocked response
or use a controlled fetch path. Implement top-level navigation denial, permission
denial and redirects through the supported pinned APIs. HTTP callbacks alone are
not a proof of blocking every scheme/DNS prefetch; test non-HTTP pathways too.

Default: no remote image, stylesheet, font, background URL, CSS @import, SVG external
reference, media, iframe, prefetch/preconnect or navigation request. Apply a strict
CSP on the actual document response: `default-src 'none'`, `script-src 'none'`,
`connect-src 'none'`, `frame-src 'none'`, `object-src 'none'`, `form-action 'none'`,
`base-uri 'none'`; add only the required message-local raster/style sources. This
policy is defense in depth, not a replacement for delegates. Do not expose a
local HTTP server, cookies, persistent storage, plugins, developer tools or a
JS→Rust command bridge to mail. Disabling JS JIT does not disable scripts: remove
active content and enforce CSP; verify the selected Servo API's execution settings.

Per-message “Load remote content” is explicit. Prefer fetch through the app's
controlled request path, with no cookies/referrer, validated TLS, redirect and
byte/time limits, refusal of loopback/link-local/private destinations at every
redirect and after DNS resolution, then supply sanitized image bytes locally.
Do not give an email unrestricted network access when this button is pressed.
Rendering privacy does not equal anonymity after a user chooses to fetch a
tracking URL; explain it briefly in that control. If safe CSS/font remote handling
cannot fit the first release, keep those blocked and document the compatibility
limit instead of enabling unrestricted Servo networking.

External http/https links require user activation and open through ASHPD's default
browser portal. `mailto:` opens the native composer through a validated URI parser
(no send action); show the true destination for misleading link text. Never launch
an arbitrary scheme/executable from email content. Attachments save through the
document/file chooser portal with safe suggested names; never execute automatically.

### Pinned API constraints for the next synthetic resource-table slice

A 2026-10-28 source audit of the locked Servo 0.7 graph found a narrow safe API
route, not a completed security boundary. Preserve the synthetic-only public
renderer while implementing these steps:

- Register one custom protocol on the retained engine. `ProtocolHandler` is
  `Send + Sync`, so its active-generation registry must use thread-safe shared
  ownership, with immutable bounded document/raster bytes. Its `load` receives the
  full `Request`, including `target_webview_id`, destination, origin and
  `current_url()`. Leave privileged paths empty, fetchability false and secure
  status false. An unguessable application namespace does not by itself prove
  browser opaque-origin or same-origin semantics.
- Resolve CID identifiers into an exact URL/resource table before publication;
  reject duplicates, ambiguous/malformed encoding, missing parts, unsupported MIME
  and excess size. Bind each table to its WebView ID and fresh generation. Retire
  the previous generation before activating another. Test the first document's
  view association rather than allowing absent IDs to bypass that binding.
- Both resource delegates must explicitly cancel non-table requests. The
  engine-wide fallback must deny unassociated requests instead of inferring a
  message from whichever view is currently selected. Test delayed requests after
  A-to-B switches and teardown; old URLs must never resolve to the new table.
- Cancel every redirect in this slice. The interceptor populates the callback's
  URL from `Request::url()` (the first URL), whereas `current_url()` is the last
  URL. The callback exposes `is_redirect` but not that current target. Do not
  implement a redirect-target allowlist using the callback's original URL.
- Do not try to replace built-in `file:`, `data:` or `blob:` handlers through the
  embedder registry: Servo installs them first, and registry merge preserves
  existing handlers. The inspected fetch path intercepts before scheme dispatch,
  consistent with the existing file/data cancellation tests. This is not proof
  that every non-fetch path is intercepted.

Keep navigation and permissions denied. Navigation requests do not establish user
activation; a portal link route needs separate input and activation tests. The
audit did not establish a public global script-disable option or an active
DNS-prefetch/preconnect bypass. Retain sanitizer/CSP layers and test script
execution, DNS, sockets and local file opens with independent positive controls.
Test actual CSP matching, custom-scheme image loading and cross-generation
isolation; a texture, callback count or URL shape is insufficient proof.

Primary pinned source: [protocol registry and handler](https://github.com/servo/servo/blob/aac43a3f31a259f04a574f5ec4e959c943ad7cc7/components/net/protocols/mod.rs),
[request interceptor](https://github.com/servo/servo/blob/aac43a3f31a259f04a574f5ec4e959c943ad7cc7/components/net/request_interceptor.rs),
[request URL accessors](https://github.com/servo/servo/blob/aac43a3f31a259f04a574f5ec4e959c943ad7cc7/components/net_traits/request.rs).
The revision is recorded by `servo-net`'s vendored `.cargo_vcs_info.json`.
No runtime, origin/CID, DNS, script or complete privacy row is checked by this
source audit.

The subsequent synthetic implementation registers an immutable bounded document
and one fixed known PNG per UUID generation, bound to the concrete WebView ID.
Both resource callbacks and the protocol handler enforce that binding; the
engine-wide fallback denies unassociated requests. The builder queues the exact
document URL, and the binding is installed before driving the UI event loop.
The local-raster subprocess exercises the initial document and image routing,
then a distinct second view with retired and unknown raster URLs. See P01
evidence for the result and its limits. The public constructor still accepts
only the fixed synthetic fixture. Generic CID resolution and browser-origin
semantics remain unimplemented/unproven; the namespace is not an origin claim.

A separate synthetic subprocess probe now distinguishes production CSP from
sanitization using an inline script and an image `onload` handler. Production
loads the permitted local raster without either script effect. A private
test-only script-permissive policy observes both effects and loopback execution
markers; sanitized input under that same permissive policy has neither effect.
See P01 evidence. This covers those two fixtures, not a global script-disable
setting or the full active-content matrix. Production CSP is unchanged.

Sources: [resource delegate](https://doc.servo.org/servo/trait.ServoDelegate.html),
[WebResourceLoad](https://doc.servo.org/servo/struct.WebResourceLoad.html),
[WebView](https://doc.servo.org/servo/webview/struct.WebView.html),
[Ammonia](https://docs.rs/ammonia/4.2.4/ammonia/).

## 4. Verification matrix

Minimum screenshot/interaction corpus: plain text, nested table newsletter,
multipart/alternative, CID image, blocked tracking pixel, dark-aware CSS, explicitly
light newsletter, Unicode/RTL, malformed HTML, very long message, large image,
phishing link, script/form/iframe payload and CSS/SVG/font/import external payload.
Commit synthetic fixtures only, with expected visible behavior/network count.

Measure GUI-only idle, one HTML reader and headless process separately. Initial
targets on reference laptop: no continuous repaint while idle, <1% of one CPU core
idle median over 60 s (network activity identified separately), reader incremental
memory <=350 MiB, no monotonically growing memory/thread count over 100 message
switches. Targets require measurement; Servo may need more memory. Record both
peak and settled memory and full process tree PSS when comparing paths.

Use a screen reader/keyboard walkthrough. Texture output by itself has no
accessible text tree. Investigate Servo's accessibility embedding support and its
bridge into libcosmic's accessibility tree. If missing, provide an explicit native
plain-text reader mode using decoded body text with accessible labels/actions,
while leaving HTML available. Record that HTML accessibility limitation; do not
call a raster texture a fully accessible document. Native mode still has keyboard
navigation, copyable text and attachments and uses COSMIC's native dark theme.

Pinned candidate, not an implemented fallback: use
`cosmic::widget::selectable_text` inside an Iced scrollable. The selected libcosmic
defaults retain `a11y`; its wrapper delegates to Iced's selectable text, which
already handles Unicode layout, focus, selection, Ctrl+A/C and copy context menus.
The inspected paragraph accessibility node exposes the whole fragment as its
label and bounds, with `Live::Polite`; it does not expose text runs, selection or
text-range actions. Test whether this meets the reader's screen-reader needs.
Do not infer text navigation from the presence of a paragraph node. Keyboard
scrolling and keeping selected text visible also need a runtime test; public
`scrollable::scroll_to`/`scroll_by` operations are candidates if explicit commands
are needed.

Do not substitute `text_editor` solely because it supports selection and copy.
Omitting `on_action` disables interaction; accepting only non-edit actions can
keep a read-only buffer interactive, but the pinned Iced editor has no
`a11y_nodes` override, and its COSMIC wrapper forwards that empty default tree.
The editable upstream example does not establish accessible read-only behavior.

Servo itself exposes `WebView::set_accessibility_active`, a subtree ID, tree-update
delegate and action forwarding. Its preference must be enabled, and the subtree
must be grafted before forwarding updates to avoid violating AccessKit invariants.
The locked Servo uses registry AccessKit 0.24.1, while Iced uses Git AccessKit
0.22.0: their Rust types are distinct. No safe public tree/action bridge has yet
been established or tested. This source mismatch does not prove that a narrow
bridge is impossible. Retain the planned real-session AT-SPI/screen-reader and
keyboard acceptance for either route, including long Unicode text and theme
changes.

Source audit, 2026-10-28: [COSMIC selectable text](https://github.com/pop-os/libcosmic/blob/8990a058e1deb8f9f7f4c0a42f6b1362fa1437ec/src/widget/selectable_text.rs),
[pinned Iced text nodes](https://github.com/pop-os/iced/blob/229186b5180b6217d93866c912e6373ac378b231/core/src/widget/text.rs),
[Iced editor](https://github.com/pop-os/iced/blob/229186b5180b6217d93866c912e6373ac378b231/widget/src/text_editor.rs),
and the vendored Servo WebView/delegate APIs. No accessibility acceptance row is
checked by this source audit.
