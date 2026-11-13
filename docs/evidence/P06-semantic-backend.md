# P06: reusable local semantic backend

2026-11-13. This slice implements the verified encoder and persistent vector
adapter in the production workspace. It does **not** yet connect semantic search
to the canonical queue, Settings or native search field. Those PLAN rows remain
open. The current application still answers lexical/fuzzy queries as before.

## Existing components, bounded glue

`mailstronaut-models` owns one explicitly loaded CPU Candle 0.11 BERT encoder,
Hugging Face tokenizers 0.22.2 and the seven-file, hash-pinned MiniLM manifest.
No custom transformer, tensor runtime, tokenizer, downloader or model service.
The lock adds 56 package versions without removing/upgrading previously locked versions.
The manifest and the unchanged frozen relevance corpora moved into the crate;
experiment paths are symlinks to the same committed data, not separate copies.

Model loading verifies sizes and SHA-256 hashes, including tokenizer, pooling and
sentence settings, before inference. Nix-style artifact symlinks are allowed only
to regular targets whose complete bytes pass those same checks. No paths/inputs
are reported in failures. The hash-verified model config selects CPU/F32, masked
mean pooling and L2 normalization, matching the independent encoder probe.

Document projection bounds text to 32 KiB at a UTF-8 boundary, uses the existing
tokenizer's 128-token windows with 32-token overlap, and evaluates at most eight
windows. An explicit `truncated` result reports omitted source text; this is not a
claim of complete analysis of long mail. Queries are limited to 1 KiB, reject
controls/empty text and reject model overflow rather than silently changing their
meaning. Model/projection policy is versioned separately from a caller's exact
source hash. Model input remains untrusted text, never instructions.

`mailstronaut-search::semantic` uses USearch's safe Rust API, F16 cosine dimension
384 and the passing search expansion 2048. Multiple windows share one canonical
message key; the adapter deduplicates messages after library retrieval and applies
allowed ID/source predicates before its bounded hit collection. Callers must get
those allowed revisions from the canonical account/folder/date/sender query.
This component does not open messages, set Seen or access their source store.

A private SQLite cache stores source hashes and normalized vectors transactionally;
it is independent derived data, not configuration or the canonical mail database.
The standard library's exclusive file lock prevents competing writers. Failed
mutations invalidate the owner, which must reopen before answering any query.
The optional USearch graph checkpoint is atomically replaced and checksum-bound
to the cache's generation. Missing, damaged or interrupted/stale checkpoints
rebuild from persisted vectors rather than repeating inference. This deliberately
retains a lossless vector cache beside the quantized graph, trading additional
bounded disk space for crash recovery without writing a new storage/index engine.
Checkpointing belongs at idle/batch boundaries, never after every message.

The cache schema/model policy invalidates old vectors on change. Graph checkpoint
size is bounded to 1 GiB and reopen refuses more than one million cached windows.
Physical power-failure injection, large-cache recovery costs, secure purge of
SQLite free pages, and canonical queue/revision/configuration acceptance remain
open. Synthetic missing/corrupt/stale-file tests are not those broader proofs.

## Explicit Nix installation

`nix build .#semantic-model` is a separate, optional hashed derivation, with the
publisher's revision/Apache-2.0 identity and the standard license text. It uses
Nix's `linkFarm` to share immutable fetch outputs rather than duplicate the large
weights. It is not a dependency of the default application package, a startup
fetch or an enabled search setting. Runtime/Settings/Home Manager model adoption
will be connected in the next slice using this same native configuration path.

## Verification

- `nix develop --command bash ci/check.sh` passed: formatting/Nix/action/ShellCheck,
  strict workspace Clippy, 315 workspace tests, 26 ignored fixtures/benchmarks.
- The explicitly invoked real-weight encoder test passed under Cranelift debug:
  fixed/held-out vector exports, empty/control/overflow query rejection, overlapping
  document windows and the eight-window/UTF-8 coverage bounds. Initial acceptance
  caught an underlength overlap fixture; it was lengthened without changing the
  encoder or either frozen relevance corpus.
- Independent local CPU PyTorch 2.14.1 / Transformers 5.17 parity passed again on
  the production encoder's actual exports. Fixed document/query maximum errors:
  1.3411045e-7 / 1.1175871e-7; held-out: 1.7136335e-7 / 1.1920929e-7. Every paired
  cosine was >=0.99999964 (unchanged gates <=1e-4 and >=0.99999).
- The production persistent-index integration test retained fixed 10/10 first and
  10/10 top-three, held-out 22/24 first and 24/24 top-three; saved/reopened results
  stayed identical. Basis-vector tests separately cover exact revision/scope limits,
  deduplicated windows, removal/replacement, no/stale/damaged checkpoints, model
  invalidation, competing writer refusal and failed-mutation owner invalidation.
- `nix build .#semantic-model` passed. Loading that actual linkFarm output through
  the production encoder passed the same explicit real-weight test; symlink
  targets remain subject to full content verification. The model output is
  `/nix/store/hzr9x59qmlw5kqa6m4qyw1fyf8gbx37x-mailstronaut-minilm-e8f8c211226b894fcb81acc59f3b34ba3efd5f42`.
- Fresh vendor staging deliberately used a dummy hash and reported
  `sha256-1Hlmuyj2k5FjGt06TIp7Tf1u5eQj6t1gEtLgYzZPEBo=`. The corrected full
  `ci/nix-check.sh` then passed on x86_64-linux, including its exact vendored-lock
  comparison, release Clippy/tests and package/fixture checks (`gate-exit=0`).
  aarch64 was not built. [Gate excerpt](artifacts/p06-semantic-backend/release-gate.log).
- Reproducible Flatpak input preparation passed with app pin
  `8d00252c02683d873691a5b9c203e0c90fc5444a6a8c4ffaf6c0d546582e686b`
  and vendor pin `0ebeac1d2d97ce07166583e4321061eb935ca4b1b19f6700edaee0b24d416009`.
  This is preparation only: the SDK/app acceptance has not been rerun for this slice.

[Workspace and independent-reference excerpt](artifacts/p06-semantic-backend/checks.log).
The complete logs stay in the ignored project target. At the end CI retained
26 GiB root / 13 GiB data free; no garbage collection or deletion was performed.
This does not repeat the encoder-only resource benchmark as a combined app claim.

No new GUI feature or screenshot is claimed by this backend-only slice. The next
native feature must include a private Wayland screenshot and actual matching-
original/Seen/filter/cancellation acceptance. The canonical queue, runtime owner,
configuration adoption, combined costs and semantic UI remain open.
