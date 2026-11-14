# P09: expired requests cannot acquire a fresh fallback lifetime

2026-11-14. The classifier-quality preparation found a concrete rules bug, separate
from model inference. An explicit “Please complete the form by 3 November 2026”
received on 4 November acquired a new 72-hour expiry. The date extractor correctly
excluded past dates from upcoming deadline boxes, but that discarded the constraint
before priority eligibility selected its no-deadline fallback. The new regression
failed on the old code. [Before-fix output](artifacts/p09-expiry/before-fix.log).

The same bounded cue/negation/quote/date extractor now retains past obligations
internally for eligibility. Public upcoming-deadline extraction and displayed date
boxes still exclude dates before authorship. Dates are interpreted at authorship,
but priority eligibility tests their instants against receipt: if every definite
obligation has already expired, it emits `AlreadyExpired` and no fallback expiry.
A valid future obligation in that same message still wins; unknown/ambiguous dates
are not converted into definite expired constraints. This is conservative when
an old explicit obligation and a new undated task coexist.

Rules policy increments from 1 to 2 so the existing bounded worker rechecks eligible
unread cached messages instead of retaining the buggy result. Dismissal/read-cleared
state remains authoritative in the canonical store. No model, new date parser,
configuration option, schema migration or dependency is added.

Verification:

- Pinned workspace format/Nix/action/ShellCheck, strict Clippy and **324 tests,
  27 ignored** passed on CI.
- Regressions cover explicit expired requests in EN/FR/IT, an already-passed time
  today, mixed past/future obligations, and mail arriving after its relative-date
  deadline. Existing quote/negation, DST, unknown date, TTL and read/dismiss tests
  pass. The initial test failure was retained; no frozen model-quality labels were
  modified to accommodate this fix.
- The full real TLS Dovecot/private native Wayland/Secret Service priority walkthrough
  passed with an extra expired questionnaire. All eight initial messages remained
  cached and unread; only the two eligible unexpired requests entered priority.
  The expired original had `expires_at = NULL` and `AlreadyExpired`. Dismiss,
  explicit-open-only Seen, unread without resurrection, new delivery, restart,
  uncertain date evidence and Show all still passed. Private screenshots are under
  `target/remote/priority-expiry-images` and were displayed in chat.
  [Workspace/native excerpt](artifacts/p09-expiry/checks.log).

The serialized x86_64 `ci/nix-check.sh` passed (`gate-exit=0`), including exact
vendor-lock equality, stable LLVM release Clippy/tests, package and private
credential/portal/instance checks. Its independent receipt
`target/priority-expiry-gate.status` is `0`; aarch64 was omitted. Package output:
`/nix/store/1yjywlc9386g3pqbqi1afmi28jdl1d68-mailstronaut-0.1.0`.
The full native walkthrough also passed against that **installed release binary**
in `target/p02.IlNhwA/priority-gui`.
[Release excerpt](artifacts/p09-expiry/release-gate.log),
[installed native run](artifacts/p09-expiry/installed-native.log).

Prepared Flatpak app sources now hash to
`7e9e0ed637273af58dba57f6f098ff0d2b3d2d1e9fb54008e70b8d5280b711bb`;
the vendor archive remains
`0ebeac1d2d97ce07166583e4321061eb935ca4b1b19f6700edaee0b24d416009`.
There is no Cargo.lock change; the lock comparison passed. Source preparation is
not an SDK/runtime claim. CI retained about 25 GiB root / 10 GiB data free, with
no cleanup required.

This is rules acceptance, not priority-model quality, real-provider or Flatpak SDK
runtime acceptance. See PLAN.md for the remaining work.
