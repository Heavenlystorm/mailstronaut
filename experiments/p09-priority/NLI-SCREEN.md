# Frozen classifier API/reference screen

2026-11-14, before downloading the classifier weights or inspecting inference.
Use the already planned MoritzLaurer multilingual MiniLMv2 L6 NLI candidate,
revision `0a71e92a985b6e1ad1828cf67ce9c459639c1dca`. The publisher's exact revision
API supplies the large-file SHA-256 values; small files were checked against their
Git blob identities before deriving SHA-256. Six files total 445,088,398 bytes
(about 424.5 MiB), below the existing 450 MiB bundle limit. MIT is the publisher's
card declaration; shipping notices remain a separate distribution requirement.

Use Candle 0.11's existing `XLMRobertaForSequenceClassification::new(3, config, vb)`
and `forward(ids, mask, zero_types)`. No custom classifier head, unsafe mmap,
architecture fork, pickle, remote Python code or production Python runtime.
The classifier computes its own padding-aware position IDs. Pair formatting and
premise-only truncation use Hugging Face tokenizers. This separate probe shares
the existing minimal encoder lock and app target, adding no production dependency.

The nine frozen synthetic pairs cover English/French/Italian, negation, neutral
text, malicious instructions as inert data, two equivalent padded forms and an
overlength premise. They are numerical checks, **not** a priority quality dataset.
Before examining outputs require exact token IDs/masks, finite three-label logits
with the validated entailment/neutral/contradiction order, maximum absolute error
<=1e-4 against offline CPU PyTorch/Transformers, <=1e-4 change from right padding,
and a 512-token bounded first window for the long premise. Keep failed results.

Record cold load, each pair's latency and fresh-process peak RSS using the pinned
release build. These preliminary CI CPU timings do not establish the full
eight-pair per-mail p95 budget or the user's laptop/battery behavior. The planned
180-message grouped development/held-out priority comparison and existing read,
expiry, confidence/language, cancellation/unload and combined-model resource
requirements in CLASSIFIER.md remain mandatory before application adoption.

Run through `ci/remote.sh`, without compiling on the laptop:

```sh
nix develop --command python3 experiments/p06-search/provision-model.py --nli
nix develop --command bash experiments/p09-priority/nli.sh check
nix develop --command bash experiments/p09-priority/nli.sh run
nix develop .#search-reference --command bash -c \
  '"$MAILSTRONAUT_REFERENCE_PYTHON" experiments/p09-priority/nli-reference.py'
```

Provisioning is an explicit development action, never app startup behavior.
The existing optional reference shell already contains the independent libraries.
Model files, compiled probe and observations stay in ignored `target/p09-priority`.
