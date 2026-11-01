# P01 — cached HTML in the native reader

Later [explicit web-link support](P01-web-links.md) supersedes this increment's
inert-link behavior. The observations below apply to its recorded source/package.

2026-11-01. Cached originals now use the passive Blitz reader, with the native
text alternative always available. **P01 and the complete client remain open.**
No real mail, credentials or provider was used for this increment.

## Implementation and ownership

One owner thread per window retains the active sanitized `Document`, font/layout
state and local image table. A Tokio watch channel coalesces pending source or
viewport requests; the reply queue holds one frame. UI callbacks only submit
owned inputs or move returned BGRA pixels into GPUI's ordinary `RenderImage`.
Source identity, reading epoch, request serial and exact viewport fence obsolete
results. Hiding or replacing the view retires its document; dropping the channels
stops the worker. No timer, per-message runtime, foreign window, custom shader or
HTML input event forwarding was added.

GPUI's standard scroll handle and Kit horizontal/vertical scrollbars own native
scrolling. A small public canvas bridge observes their actual bounds/offset and
paints only a matching bounded physical viewport. Home/End/Page Up/Page Down
operate on that same handle. Logical geometry, scale and theme enter Blitz's
public viewport API; scroll-only paints reuse the immutable layout. Native
dimensions/scales/offsets are validated before narrowing or allocating. Existing
HTML/DOM/resource/raster limits remain unchanged; these are allocation/API bounds,
not a hard process-memory or CPU-time cap.

Kit's segmented TabBar and Toolbar provide HTML/Text and Select text. The
existing read-only Textarea owns text selection/copy; a raster is not presented as
an HTML accessibility tree. Plain-only mail disables HTML. Unsupported HTML or
invalid native geometry falls back to the cached text. Invalid or ambiguous MIME
image extraction and invalid raster signatures/budgets omit local images with a
visible notice rather than hiding the whole message. No remote resource fallback
is attempted. CSS CID backgrounds, SVG/GIF and interactive HTML remain unsupported.

The cached MIME projection reaches the owner thread through the existing bounded
store command. `Projection::inline_rasters` supplies transfer-decoded local data;
the renderer independently validates it. Only explicitly opening the selected
original authorizes Seen. HTML must actually paint a current frame before that
authorization can be consumed. Text fallback can consume the same authorization;
passive focus, resize/theme, refresh, restored pages and mode changes cannot create
one. The existing query/body/flag generations remain independent.

## Development checks

Through the pinned `nix develop` environment:

```sh
cargo fmt --all
bash ci/check.sh
cargo build -p mailstronaut --locked --offline
```

Formatting, Nix/CI/shell lint, strict all-target workspace Clippy and **144 normal
tests** passed. The three new behavior tests verify actual MIME image painting
and A→B isolation, release of the hidden worker's source and channel shutdown,
coalescing 100 queued requests to the latest bounded physical frame, and native
geometry/scale/offset/allocation boundaries. Earlier MIME, retained viewport,
resource-policy and Seen tests continue passing. No dependency/lock/vendor hash,
compiler exception or application runtime input changed. Strace is a GUI-test
tool only.

The initial native demo also passed its existing light/dark, real local raster,
read-only Unicode copy/edit rejection/navigation and X11-only rejection checks.
The cached-message fixture then exercised four distinct originals in two private
accounts, without contacting an account server.

## Native package acceptance

The coordinated `ci/nix-check.sh` service used all 16 available CPUs, one Nix job,
stable LLVM, thin LTO and one codegen unit. Strict release Clippy, **144 normal
tests and five private credential/OAuth/account-action checks** passed. Service
`mailstronaut-native-html-gate.service`, invocation
`1e3f4f2627f64261b714a0c8b262925b`, finished with a fresh numeric receipt `0`, a full
`all checks passed` log, inactive/collected state, `MainPID=0` and
`ExecMainStatus=0`. Aarch64 was omitted. No second full release build ran alongside
it and no hosted CI wait was used.

The installed package is
`/nix/store/bw70m14m72psn8wkl1vnwxk6xr3s3r2p-mailstronaut-0.1.0`.
All 132 files in its filtered source
`/nix/store/8lmdj37wn650z21i0q7dgxkwaqgcdpp8-source` match the checkout byte for byte.
Installed empty-environment version, native config init/check, explicit example
evaluation and private config permissions passed. The native fixture is
[reader.sh](../../experiments/p01-blitz/reader.sh), run in `nix develop .#gui-test`
against this exact installed binary with the prebuilt synthetic core fixture.

It runs with private XDG paths and activation-free D-Bus, DISPLAY unset, Sway's
XWayland disabled and pinned software Vulkan. Four cached synthetic originals in
two accounts establish:

- Passive list selection/refresh leaves every original unread. Enter/open marks
  only the loaded original; body-mode, theme, resize and Settings restoration do
  not manufacture a new authorization. SQLite independently observes effective
  per-original Seen state.
- Alpha's actual multipart MIME image paints ordered RGB bands. Native End/Home
  paints the tail/top, a 960-pixel resize changes the CSS media-query marker, and
  2x native scale doubles physical image height and band spacing while retaining
  its CSS size. Dark chrome and Blitz dark-media pixels are both observed.
- Beta references Alpha's CID without owning it. Its distinct own-frame marker
  must appear before assertions accept the absence of Alpha's image. This also
  passes after eight A→B cycles and hiding/restoration. A corrupt local PNG still
  leaves its own HTML visible with an omission notice. Plain-only mail uses Text.
- The native text reader copies each original's exact alternative, including
  French/CJK Unicode and the plain-only message's retained CRLF. Clipboard offers
  are observed with a deadline; a fixed 200 ms delay was unreliable under tracing.
- A real loopback HTTP request and readable synthetic file prove the independent
  strace observers work. Following the application and its children across the
  whole initial/offline session observes zero IPv4/IPv6 socket attempts, zero
  opens of the email-specified synthetic file and no additional server requests.
  CSS import/background/font, images, frame/script markup and a clicked inert link
  are included. This native fixture is not a separate DNS/scheme corpus; the
  broader renderer API matrix remains in the preceding evidence.
- The app exits normally, restarts offline, and reopens Alpha's cached HTML with
  its own image while preserving canonical Seen state.

Installed fixture: `target/p01.ig5MSp`, exit zero. Screenshot pixels are asserted
independently, not inferred from input events or a window existing. Image
upsampling blends band edges, so the assertions check exact solid interiors,
physical height/spacing and distinct current-frame markers. Early harness runs
exposed the clipboard timing and an overly strict unblended-width expectation;
those observers were corrected before the complete installed run passed.

- [Native fixture output](artifacts/p01-cached-html/native-reader.log)
- [Installed CLI/config smoke](artifacts/p01-cached-html/installed-cli.log)
- [Independent observer receipt](artifacts/p01-cached-html/native-observations.json)
- [Source/gate identity and file hashes](artifacts/p01-cached-html/source-and-gate.json)
- [Release check excerpt](artifacts/p01-cached-html/release-excerpt.log)
- [Dark cached HTML](artifacts/p01-cached-html/native-dark.png)
- [Narrow layout](artifacts/p01-cached-html/native-narrow.png)
- [Document end](artifacts/p01-cached-html/native-end.png)

Full traces/logs remain in ignored project `target/`. The actual development
cached-HTML screenshot was shown in chat; supplied reference images remain
ignored and are not committed.

The installed preview/native-text regression also passed in
`target/gpui-wayland.6MNrPy`: light/dark HTML and local-image pixels, exact long
Unicode copying, rejected edits, document-end navigation/selection preservation
and X11-only startup rejection. The final current-script `nix flake check` and
cached `nix build` passed without another application compilation. This rechecks
the final fixture's shell lint after its observer adjustments.

- [Installed regression and final cached checks](artifacts/p01-cached-html/final-checks.log)

## Usability and remaining work

The current actual reader was compared with the retained Spark reference for
navigation/list/reader hierarchy and available reading space, including a narrow
960-pixel window. Kit supplies the controls, theme and scroll behavior; the email's
own layout remains inside the reader. The reference is not shipped or copied.
The client still lacks daily-use essentials already tracked in P06/P07: complete
account/folder navigation, timestamps/thread-original controls, composition,
attachments and routine actions. This increment does not claim daily-driver parity.

The malformed nested table's leading text has poor contrast in the dark-media
fixture with these pins; the native Text alternative remains readable. Fixed
author-supplied colors are retained, so the fixed-light Beta message remains white
inside dark native controls. No custom email color inversion or upstream table
fork was introduced. Broader table/font/layout compatibility still needs a corpus.

This is bounded synthetic acceptance, not a comprehensive hostile-email audit,
layout corpus, fuzzing, screen-reader/AT-SPI proof, HTML selection, explicit portal
links, hardware-GPU or performance/memory/idle acceptance. Flatpak, aarch64 and
live-account acceptance remain separate. The isolated renderer API's broader
DNS/scheme/file matrix is recorded in [P01-passive-viewport.md](P01-passive-viewport.md).
