# P09: multilingual classifier quality gate

2026-11-14. **Candidate rejected; no application inference is enabled.**

The exact MIT six-layer MiniLM NLI artifact and Candle port passed full-logit
reference, padding, token-budget and isolated safe-loader memory checks. Task
quality was evaluated separately, using the frozen grouped corpus and policy in
[QUALITY-SCREEN](../../experiments/p09-priority/QUALITY-SCREEN.md). All computation
ran on the CI CPU in pinned Nix environments. No mail, model input or query was
uploaded to a model service. The explicit provisioned artifact stays in CI target.

The original 180 synthetic messages comprise 30 semantic families, translated
into EN/FR/IT with two presentation variants. An additional six semantic families
(18 rows) were frozen as held-out context challenges before any quality scoring.
Development contains 108 messages and held-out 90. These correlated counts are
not independent live-mail samples. Family, sender and thread stay in one split;
model input excludes the synthetic identifiers and gold labels. Development
filtering precedes all encoding.

Initial metatextual p1 hypotheses and strict three-way thresholds produced zero
combined corrections. An elementary NLI control established correct label order.
A predeclared nine-development-message diagnostic selected simpler second-person
hypotheses. Their inherited thresholds also produced zero combined corrections.
One registered calibration used the maintained independent-label entailment/
contradiction score, retaining three-way entailment/neutral guards. It corrected
seven development false negatives without a new false positive. The final p2
policy was committed before any held-out encoding, and was evaluated once.

| Held-out language | Rules TP/FP/FN | Combined TP/FP/FN | Combined precision/recall | Accepted deadlines |
| --- | --- | --- | --- | --- |
| English | 14/2/3 | 14/2/3 | 87.50% / 82.35% | 12/14 |
| French | 14/2/3 | 15/4/2 | 78.95% / 88.24% | 12/14 |
| Italian | 15/4/2 | 17/7/0 | 70.83% / 100% | 12/14 |

The candidate adds three correct positives and five false positives. Macro F1
decreases from 0.8434 to 0.8370; French/Italian combined precision and accepted
deadline precision fail the unchanged gates. The original subset alone has
Italian precision 14/18 and the model fails the no-increase-in-false-positives
criterion. The held-out evaluator writes outcomes then returns **exit 1** and
`heldout-quality-gate=fail quality=False benefit=False`. This is an actual failed
acceptance, not a tool/CI failure. No thresholds, labels or splits were changed
afterwards. The current rule parser's contextual date limitations are also now
explicitly reopened in PLAN.

CPU warm p95 was 247.317 ms for the four-pair held-out policy, with no truncated
fixtures. Isolated peak was 859264 KiB, steady 726760 KiB (about 710 MiB); this is CI, not a laptop or combined-search/process measurement.
Reference arithmetic, resource feasibility, synthetic quality, application worker
lifecycle and distribution acceptance are separate. No lazy application NLI
worker, model import/optional package or combined-memory claim is made.

[Initial p1](artifacts/p09-quality/p1-development.log),
[wording diagnostics](artifacts/p09-quality/wording-diagnostic.log),
[p2 inherited](artifacts/p09-quality/p2-inherited-development.log),
[p2 development calibration](artifacts/p09-quality/p2-calibrated-development.log),
[all held-out outcomes](artifacts/p09-quality/p2-heldout.log).

No weights or raw real mail are committed. The application remains the already
validated rules-only implementation. A future candidate must use a new frozen
held-out extension; these failed cases may become explicit regression cases,
not fresh held-out evidence.
