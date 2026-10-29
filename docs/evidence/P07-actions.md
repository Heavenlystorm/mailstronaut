# P07 — Durable read/star actions and unsent undo

2026-10-29, x86_64-linux. This extends the receiving/compose prototypes. Full P04,
P06 and P07 acceptance remains open. All protocol and native fixtures are synthetic.

## Behavior

- Native cached-reader controls mark read/unread and star/unstar. Intentions commit
  on the dedicated SQLite owner and immediately overlay every inbox membership
  of the selected canonical message. Sync's observed flags cannot erase pending
  local intentions. No other account is affected. Opening alone still leaves the
  flag unchanged; downloading/classification always remains PEEK-only.
- Apply flags explicitly authenticates through the existing scoped keyring/OAuth
  session and processes at most 20 assignments/account, at most four accounts at
  once. Accounts without queued work are not contacted. There is no automatic
  background drain, scheduler, new connection pool or mail protocol implementation.
- Each operation retains account, canonical message, exact folder, UIDVALIDITY,
  nonzero UID, desired value, revision and immutable attempt token/snapshot.
  Claiming commits before network handoff. Rapid opposite edits coalesce without
  overwriting a submitted snapshot; old acknowledgments cannot delete newer work.
  Discarded accepted edit/result waiters do not drop durable writes. A claim whose
  waiter disappeared is failed rather than leaving the account permanently busy.
- The authenticated incoming configuration's public fingerprint is stored with
  cached UID namespaces. It includes endpoint/security, principal/address, auth
  metadata and configured trust paths, without credential values. Changed server
  or authentication identity retires old bindings/operations and discards paging
  positions while retaining originals and user state. Legacy unknown-provenance
  UIDs require a successful authenticated sync before flag editing. A new account
  configuration cannot attach its fingerprint to arbitrary old cached UIDs.
- Before each UID STORE, SELECT must report the same nonzero UIDVALIDITY. The
  library also must positively report the chosen flag in PERMANENTFLAGS. Its empty
  vector conflates absent and explicitly empty support; this prototype conservatively
  refuses both, so session-only flags are never accepted as permanent changes.
- Async-imap owns wire commands/parsing. Assignments use only numeric UID and one
  fixed +/-FLAGS.SILENT expression for Seen/Flagged. They preserve other flags and
  keywords, never use sequence numbers or EXPUNGE. Fresh UID metadata confirms the
  requested flag; STORE OK alone is insufficient. UIDVALIDITY mismatch stops the
  batch and retires its claim; Sync retires the remaining old namespace.
- Network/refusal/missing confirmation becomes an explicit failure; no automatic
  retry runs. Another Apply retries idempotent flags. This path never touches SMTP
  submission/delivery uncertainty. Only the exclusive application owner recovers
  interrupted flag attempts, preserving the distinct SMTP recovery rules.
- Undo unsent flags removes the selected message's unattempted intentions only.
  It refuses while any target is in flight or might already have reached the
  server, including a failed attempt later superseded by another local edit.
  Read-cleared priority history is monotonic even after unread/undo.
- Schema 4 extends the existing pending-operation table and account provenance.
  Forward migration retains old drafts, originals, reminders and opaque legacy
  rows, creates a private schema-3 backup and validates foreign keys. Configuration
  retirement keeps active task counts until completion; normal Quit cancels and
  waits for flag result bookkeeping before storage shutdown. Stale results refresh
  the current filtered snapshot without replacing current status/body selections.

## Checks

Pinned `nix develop`, nightly Cranelift/Wild with the existing LLVM exceptions:

```sh
cargo fmt --all
bash ci/check.sh
cargo build -p mailstronaut --all-features --locked --offline
```

Strict workspace Clippy, formatting and tooling checks passed. **101 workspace
tests plus seven isolated Servo child invocations passed**. See
[development-check.log](artifacts/p07-actions/development-check.log).

New store/worker tests cover offline overlays/restart/account isolation, opposite
edits during a submitted attempt, late callback fencing, explicit retry, changed
servers with identical numeric UIDs, exclusive-owner recovery, unsent-only undo,
sticky read history and schema-3 provenance/backup preservation. TLS tests execute
real library UID STORE and subsequent FETCH against localhost: success, refusal,
changed UIDVALIDITY, lost acknowledgment then explicit retry, and missing permanent
flag support. Other UID state remains unchanged; namespace/permanence failures
produce no STORE. Existing TLS/auth, MIME, draft/outbox, sync and renderer tests pass.

## Native fixture acceptance

Run the existing private encrypted-keyring harness with prebuilt application,
`imap_tls` and `credentials` executables, selected by
`MAILSTRONAUT_INBOX_GUI_BINARY`, `MAILSTRONAUT_IMAP_FIXTURE`,
`MAILSTRONAUT_CREDENTIAL_FIXTURE`, and `MAILSTRONAUT_FLAGS_GUI=1`:

```sh
nix develop .#gui-test --command bash experiments/p02-accounts/credentials.sh
```

The final X11/Xvfb 100% run passed. It synced the synthetic inbox, queued two
read intentions, verified they existed, and undid them using the native button.
It then queued read/unstar for both inbox copies, verified the observed server
flags were still unchanged, and closed normally. Light-mode offline restart kept
all four intentions. Foreground keyring unlock and Apply flags sent exactly four
UID assignments over the private TLS fixture, confirmed both flags and cleared the
queue. Another offline restart retained the confirmed read/unstar state. No SMTP,
EXPUNGE, real account, user bus/keyring or real recipient was used. The private
credential lifecycle and unavailable-service checks also passed afterward.

- [GUI log](artifacts/p07-actions/native-gui.log)
- [Independent server receipt](artifacts/p07-actions/native-server.log)
- [Queued local flags](artifacts/p07-actions/native-queued-flags.png)
- [Offline pending state](artifacts/p07-actions/native-offline-pending.png)
- [Confirmed state](artifacts/p07-actions/native-confirmed-flags.png)
- [Further offline restart](artifacts/p07-actions/native-confirmed-offline.png)

This is local development/native fixture evidence, not current-source Nix release,
CI-container, Flatpak, live-provider, every filesystem crash boundary, suspension,
Wayland or accessibility acceptance. Folder navigation beyond INBOX, persistent
sync cursors, IDLE/reconnect, automatic read/application, archive/trash/move, Sent
APPEND, reminders/background and priority classification remain open. The existing
hostile IMAP literal allocation gap remains; no full protocol-boundary claim is
made. Arbitrary email HTML remains excluded from the synthetic Servo renderer.

## Provider identity follow-up

Review of the account-provenance boundary found that retiring UID memberships
also needs to retire provider-ID mappings. Otherwise a different authenticated
mailbox reusing a numeric provider ID could replace an earlier original and inherit
its local reminder/read identity. The follow-up removes only that account's provider
mappings in the same context-change transaction. Originals and user state remain.
A new synthetic regression reuses both UIDVALIDITY/UID and provider ID on another
server and verifies distinct canonical messages, unchanged old content and its
retained reminder. The seven focused flag tests and strict core all-target Clippy
passed through `nix develop`; see
[provider-identity-check.log](artifacts/p07-actions/provider-identity-check.log).
The earlier complete development/native run above precedes this follow-up. The
in-progress release gate started at `962050d3ce4cd375e8f55010935ce211164e0f05`
and does not include it.
