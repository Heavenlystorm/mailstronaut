# Local priority analysis and deadlines

P09 implements this design. Everything in this document is a target or researched
candidate until a benchmark artifact proves it. Classification never uses an API,
remote model endpoint, external LLM service or mail upload. It runs for every newly
fetched inbox message, within the body budget, independently of opening that mail.
Deleted, unreadable, encrypted, oversized and unsupported inputs receive an explicit
status, not a fabricated prediction. Initial backlog processing is separately
opt-in and bounded; it must not monopolize interactive work.

## 1. First candidate and why

Evaluate **MoritzLaurer/multilingual-MiniLMv2-L6-mnli-xnli** at exact Hugging Face
revision `0a71e92a985b6e1ad1828cf67ce9c459639c1dca`. Its configuration is
`XLMRobertaForSequenceClassification`, six layers, hidden size 384, 12 attention
heads, 512-token input after special-token accounting. The API metadata reports
106,995,075 F32 parameters (plus 514 I64 entries); weights alone are approximately
408 MiB. This is a small encoder, not a token-generating 0.6B–35B language model.
MIT model card; language behavior still needs task-specific validation, especially
Italian (not one of the listed XNLI evaluation languages).

Use `candle_transformers::models::xlm_roberta::XLMRobertaForSequenceClassification`,
which already exposes the classifier head, with candle-core/nn/transformers 0.11,
CPU F32 and Hugging Face tokenizers. Do not implement its transformer/head, use
remote Python custom code, convert through an unreviewed runtime or assume any
MiniLM embedding model is an NLI classifier. Load config/tokenizer/safetensors
from a verified local manifest. Use a safe owned-buffer loading API first;
memory-mapped unsafe code is forbidden in our crates by the workspace policy.

The model's labels are **0 entailment, 1 neutral, 2 contradiction**. Read/validate
id2label from the pinned config; many NLI models use a different order. Construct
premise/hypothesis pairs using tokenizer pair encoding and the correct special
tokens, masks, padding and zero token-type IDs. Compare full logits against known
reference tensors before evaluating product quality. Check padding/position IDs,
softmax axis, maximum position bounds and CPU numeric tolerance. A successful load
does not establish correct NLI scores.

Primary evidence: [model card](https://huggingface.co/MoritzLaurer/multilingual-MiniLMv2-L6-mnli-xnli),
[pinned config](https://huggingface.co/MoritzLaurer/multilingual-MiniLMv2-L6-mnli-xnli/blob/0a71e92a985b6e1ad1828cf67ce9c459639c1dca/config.json),
[Candle classifier API](https://docs.rs/candle-transformers/0.11.0/candle_transformers/models/xlm_roberta/struct.XLMRobertaForSequenceClassification.html).

## 2. JevBench research is discovery, not email validation

Inspected the user-suggested board and its public JSON API on 2026-10-27. The live
API reported **v1.7.11**; it carries older per-model measurements. The public result
contains the original measured CPU/GPU condition and an adjusted latency estimate.
Do not turn GPU loopback timings, hosted prices or its generic decision score into
a laptop mail-classification benchmark. The live HTML was too large for the text
fetcher; collaborative snapshot/evaluation both timed out. The JSON endpoint was
successfully fetched with the system TLS client; selected evidence is recorded in
RESEARCH.md. No model weights were downloaded or benchmarked here.

| Candidate family inspected | Decision for first implementation |
| --- | --- |
| Multilingual MiniLM NLI ~107M | Primary: existing Candle implementation, multilingual text pairs, bounded one-pass encoder; not claiming a JevBench rank for this model |
| Quyet-1.0-Small-EN, ModernBERT fixed-head encoder | Interesting compact English comparison from the board, but its typed-decision heads/encoding and language coverage are a different integration contract; do not substitute based on rank |
| Phocinae-Largha-150M-v1 / ModernBERT decision encoders | Similar parameter range but specialized decision encodings; check card, license, inference code and Rust architecture compatibility before adding |
| Kev 0.6B / Qwen-derived 0.8B decisions | Larger weights/runtime and different task; not baseline dependencies, despite some fast GPU rows |
| Fastino GLiNER-2.5-Decide hosted row | Hosted endpoint is excluded; that row is not proof of an equivalent downloadable local model |
| Rule baseline | Required comparison and fallback when model absent/disabled; direct-recipient + action/deadline cues, no claim of full semantic understanding |

Do not ship a model merely because its name says “decision”. Any alternative must
fit the same typed result contract, local privacy rule, no custom Python, and CPU
budget, and beat the rule baseline on held-out email fixtures. Prefer reducing the
number of NLI hypotheses/windows before increasing parameter count.

## 3. Input, output and contextual TTL

Classifier input is decoded subject, sender/recipient metadata and normalized visible
body text. MIME parsing belongs to the existing mail parser. Exclude scripts, CSS,
invisible tracking text and safely identified quote/signature tails from analysis;
keep the untouched original for reading. Treat sender/header content as untrusted
features, not authoritative user identity. Never fetch links or attachments for
classification. Text snippets and labels are data, never instructions to an agent.

Bound normalized text to 32 KiB, then at most two 512-token pair windows selected
from the beginning and action/date candidate context. Preserve original byte-span
mapping before whitespace normalization. Reserve tokens for the subject and
hypothesis; truncate premise only at tokenizer boundaries. If a relevant deadline
occurs outside model windows, deterministic candidate parsing may still show it
with lower confidence, but never claim the whole body was semantically analyzed.
Bound sentences/date candidates (initial 32/16) and inference pairs (initial 8 per
message); fail/defer instead of unbounded looping over a huge newsletter.

Initial fixed NLI hypothesis set, translated/reviewed for EN/FR/IT fixture coverage:

1. “This email asks the recipient to take a concrete action or reply.”
2. “This email contains a time-sensitive obligation for the recipient.”
3. “This email is an informational newsletter or advertisement without a personal obligation.”
4. “This email requires attention within the next day.”

Run fixed versioned hypotheses, not model-generated prompts/summaries. Use the
three-way probabilities including neutral for abstention; never present uncalibrated
softmax as trustworthy confidence. Rules may increase the action prior for direct
recipients and accepted deadlines, but a mailing-list action can still matter and
a direct message can still be low priority. The model is an assistant to local
ranking, not an anti-spam engine. Do not auto-train on the user's mailbox.

Typed output contract:

```text
AnalysisResult {
  body_hash, input_revision, model_manifest_hash, policy_version,
  status: Complete | RulesOnly | Unsupported | TooLarge | Failed,
  action_score, urgency_score, informational_score,
  priority: bool, reason_codes: [...],
  ttl_class: OneDay | ThreeDays | SevenDays | Deadline,
  deadline_candidates: [DeadlineEvidence],
  analyzed_at
}
DeadlineEvidence {
  original_span: [start_utf8_byte, end_utf8_byte), quote,
  local_date, optional_local_time, zone, zone_source,
  resolved_instant_or_exclusive_end_of_day,
  evidence_kind: ExplicitDate | RelativeDate,
  action_association_score, precision, user_override
}
```

Core owns final eligibility/read state and expiration, not the tensor worker. It
validates ranges, finite scores, matching hashes and source spans before storing.
Initial **provisional** decision thresholds: action >=0.70 and (urgency >=0.65 or an
accepted actionable deadline); require an adequate entailment-vs-neutral margin
calibrated in the experiment. Ambiguous cases abstain. Do not add priority to every
unread direct message. Tune thresholds once on development data, freeze on held-out
acceptance data, report coverage/abstention as well as precision/recall.

TTL policy: accepted deadline wins; otherwise action+urgent → 24 hours, normal
action → 72 hours, explicit longer-lived important obligation with supported
context → at most 168 hours. If no reliable long-lived context detector exists,
do not select the seven-day class. Configuration caps bound TTLs without moving a
deadline later. Store the actual fixed expiration anchored to received_at. For
ordinary TTL use min(contextual TTL, configured cap); a verified future deadline
may be longer than seven days and expires on its actual date. Do not truncate an
explicit real deadline because a no-deadline fallback cap is shorter. Already-read
or already-expired messages remain in the ordinary inbox, never linger in priority.

## 4. Deadline extraction without a generative model

Use bounded regex candidate discovery + `interim` for existing English relative
date interpretation and `chrono`/`chrono-tz` for timestamp/timezone arithmetic.
Interim parses supported date phrases, not whole email semantics and not arbitrary
French/Italian prose. Provide small table-driven EN/FR/IT month/weekday/relative-day
and action/deadline cue normalization, with explicit supported patterns and tests.
This is a deliberately bounded adapter, not a new natural-language parsing engine.
Unsupported/ambiguous language/date forms produce no definite deadline box.

First supported forms: ISO date, day + localized month [+ year], unambiguous
day/month/year with configured locale, today/tomorrow equivalents, weekday with
explicit relation, and optional time/offset/IANA zone when present. Recognize
deadline cues such as “submit by”, “due”, “before”, “date limite”, “avant le”,
“scadenza”, “entro” in the same sentence. Confirm action relevance using the
existing NLI pairs within the total pair budget. A flight date, birthday, invoice
issue date, quoted old deadline or “no action required” does not automatically
become a deadline. “Not due tomorrow; due Friday” must not select tomorrow.

Relative dates use the message's credible authored timestamp and explicit offset,
with account timezone fallback, **not the day sync/inference happens**. Compare the
Date header to server INTERNALDATE; if missing or wildly inconsistent (initial
48-hour discrepancy tolerance), use INTERNALDATE and mark the assumption. Mail
arriving two days late with “tomorrow” must not create a deadline tomorrow from
the current machine clock. Respect explicit year; do not roll a past year into
the future. Missing year is accepted only if contextual/receipt anchoring resolves
it without silently moving a passed obligation to next year.

Date-only inclusive end uses next civil midnight in the resolved timezone. DST
days have 23/25 hours. Timezone missing → configured account/global zone, visible
as assumed. Ambiguous abbreviations/numeric dates, DST gaps and folds are not
silently guessed: show an uncertain candidate only in the reader for confirmation;
do not use it for automatic priority expiry. Test leap days and timezone changes.
Persistence stores the originally resolved zone/instant; changing display timezone
does not shift an existing obligation. User edits win over subsequent model runs.

## 5. Model installation and execution

No automatic download. Nix may expose a separate optional `priority-model` fixed-
output derivation; it is not a dependency of the core app package. Flatpak ships
without weights and offers explicit import/download of the approved artifact set.
Use a versioned manifest with repo/revision, license, architecture, weight dtype,
file names, individual SHA-256 hashes and total size. Include config.json,
tokenizer.json, tokenizer metadata as required and safetensors; exclude pickle
weights and executable model code. Release hashes must be measured from the actual
artifacts in P09, never invented in this plan. Offline import is supported.

The mail engine may download only this allowlisted artifact set after consent;
the classifier crate has **no HTTP dependency**. Bound download bytes, verify before
atomic install, retain previous valid set on failure. No Hub telemetry/inference
API or trust_remote_code. Model updates are explicit and versioned, with no
automatic historical mailbox reclassification or read-state resurrection.

One low-priority CPU worker/job at a time; no GPU runtime in default build. Limit
underlying CPU thread use where supported, measure actual utilization and never
promise a hard limit unsupported by Candle/gemm. Load lazily, unload after 60 seconds
idle/headless. Cache decisions by body+model+policy version. Suspend backlog on low
battery if desktop power status is available; unknown power state must remain
bounded. Stop promptly on disable/account removal/shutdown. Prioritize mail reading
and reminders over classification; a model failure cannot stop mail sync.

## 6. Acceptance experiment

Create at least 180 synthetic or explicitly consented/anonymized messages: at least
60 EN, 60 FR and 60 IT; label priority, action, deadline text, normalized deadline,
and expiration. Split by sender/template/thread into development and held-out sets
before tuning. Include newsletters, questionnaires, receipts, invoices, academic
announcements, multilingual mail, noisy HTML, delayed arrival, negation, multiple
dates, uncertain dates, no year, expired obligations and quoted messages. Do not
commit real mail or credentials. A small synthetic set is a regression gate, not
a universal accuracy guarantee.

Compare rules only vs the selected encoder with identical inputs. Report priority
precision/recall, false-positive rate, per-language coverage and abstention,
deadline exact match/false deadline rate, read-clear and expiration correctness.
Initial release targets: priority precision >=0.85, recall >=0.65 on the held-out
fixture set; accepted deadline precision >=0.95 with coverage reported; zero
read-cleared/expired messages in priority. If a language fails, keep its uncertain
cases rules-only/unhighlighted and disclose the limitation, rather than advertise
full semantic support. Revisit a candidate only when data justifies it.

Measure on the user's laptop **CPU**, record CPU/thread/RAM/OS/build/model hashes,
cold load, warm p50/p95 per mail, batch throughput, peak process PSS/RSS, idle CPU,
and battery implications. Initial budgets: model bundle <=450 MiB, warm p95 <=2 s
for the bounded input/pair policy, total incremental ML peak <=900 MiB, idle weights
released, no resident GPU allocation. These are gates, not measurements. If missed,
reduce pairs/windows or run rules-only while investigating. Do not silently upgrade
to a larger model, quantize away a retained baseline, or report a GPU benchmark as
proof that these CPU budgets passed.
