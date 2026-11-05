# P01 — Native responsiveness, memory and hidden idle

Recorded 2026-11-05 on `ci-py` (x86_64). Harness: `experiments/p01-blitz/performance.sh`, run
inside `nix develop .#gui-test` against the **release** application binary. Raw numbers:
[performance.json](artifacts/p01-performance/performance.json).

## Scope of the measurement

- One synthetic long HTML original (1,500 short paragraphs, each with its own layout box) and 100
  short HTML originals. Each HTML original begins with a block of its own colour, so every paint is
  identified from a native pixel sample. No real mail, account, keyring or network service is used.
- A private headless sway session with software rendering (`pixman`/`llvmpipe`), a private session
  bus and the application's real keyboard path (`Ctrl+L`, `Down`, `Up`, `Return`).
- Latency is measured from sending a key to the first sample that shows the expected colour. Each
  sample is a `grim` capture of one pixel, which costs a median of 16 ms here (p95 16.5 ms). Every
  latency below includes that polling floor, so the figures bound the real paint time from above.

## Results

| Measurement | Result |
| --- | --- |
| Keyboard switches across 100 originals | 100 of 100 painted the expected original; 0 wrong paints |
| Switch to the next original, median / p95 / max | 73.9 / 76.7 / 91.3 ms |
| Explicit open of the long original from a short neighbour (5 rounds), median / max | 107.4 / 110.1 ms |
| Resident memory after the first long open | 261,488 KiB |
| Resident memory after 100 switches | 262,068 KiB |
| Growth from 10 to 100 switches | 196 KiB |
| Hidden lifetime: window closed with background operation enabled | process still running after 2 s |
| Idle CPU while hidden | 0.0 s over 30 s |
| Resident memory after the window closed | 231,084 KiB |

## Defect found and fixed

The first run grew by about 2.2 MB on every switch: 262 MB after the first open became 477 MB after
100 switches. GPUI keeps each rendered image's texture in the sprite atlas until `drop_image` is
called. The HTML reader replaced or cleared its frame without releasing the old image, so every
switch left a texture behind.

`crates/mailstronaut/src/html_reader.rs` now queues each replaced or cleared frame and releases it
in `render` with the current window (`cx.drop_image(image, Some(window))`). After the fix, growth
over 90 switches was 196 KiB, and the run above is the plateau evidence.

## Regression

On the same source tree, the debug build passes the existing P01 cached-HTML walkthrough
(`experiments/p01-blitz/reader.sh`): own-message pixels at every scale and theme, retired-source
checks, resize/scroll/HiDPI, the Seen gate, copy routes, and the observer network and file checks.

## Limits

- Software rendering in a headless session. These are not desktop GPU, power or thermal numbers.
- One scripted session of 100 switches and one 30-second idle window. Between samples the resident
  set rises by at most 64 KiB, with no sustained trend. Why it stays near 231 MB after the window
  closes is not yet attributed, and it was not investigated further.
- Hidden idle was measured with background operation enabled, where closing the window keeps the
  services running. With background operation disabled, closing the window exits the process.
- The CJK glyphs in the synthetic subject render as boxes in this headless font set. That is a font
  coverage limit of the CI environment, not a layout result.
- Not established: real desktop compositors, hardware-GPU drivers, multi-hour use, a 10k-message
  mailbox, and assistive-technology behaviour (see the accessibility row).
