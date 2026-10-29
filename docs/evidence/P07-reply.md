# P07 — Reply/reply-all and explicit text forwarding

2026-10-29, x86_64-linux. This adds a bounded native reply path to the earlier
compose/outbox prototype. Whole P07 remains incomplete. All fixtures are synthetic.

- Cached MIME projections retain bounded Reply-To with a backward-compatible
  Serde default. The original is parsed again on opening; no schema migration or
  downloaded message rewrite is needed.
- Reply prefers Reply-To, then From; replying to one's own sent mail targets its
  original visible recipients. Reply-all also retains visible To/Cc, excludes the
  receiving account's primary/alias identities and duplicates, and never copies
  Bcc. Other recipients' local-part case is preserved. The original receiving
  account is selected; configured alias spelling is used for sender validation.
- Lettre owns recipient/header formatting and MIME encoding. Replies retain valid
  bounded References and one direct In-Reply-To; invalid ancestry tokens are
  omitted and oldest history is trimmed without slicing identifiers. Forwarding
  starts a new thread with no recipients or inherited ancestry. Its visible banner
  explicitly says attachments are not included.
- Quotation uses the existing decoded plain alternative or inert HTML-to-text
  projection. No email HTML reaches Servo. Encrypted/incomplete/oversized input
  stops automatic preparation with a static error. Headers, recipient counts,
  ancestry and quoted text retain the existing composer budgets.
- Native reader buttons create a new editable local draft. Switching first awaits
  the current unsaved snapshot; failure leaves the editor intact. The new draft
  uses the existing autosave, normal-close and restart path. Only a later explicit
  Send can freeze or submit it.

## Development and native GUI checks

Pinned `nix develop`, nightly Cranelift/Wild and the four existing LLVM overrides:

```sh
cargo fmt --all
bash ci/check.sh
cargo build -p mailstronaut --all-features --locked --offline
```

Formatting, strict workspace Clippy and tooling checks passed. **91 workspace
tests plus seven isolated Servo child invocations passed**. Six new domain tests
independently parse generated MIME to verify Reply-To/quoted Unicode/alias/ancestry,
reply-all self exclusion/deduplication, own-sent replies, forward/Bcc isolation,
incomplete/encrypted/oversized rejection and old cached metadata compatibility.
See [development-check.log](artifacts/p07-reply/development-check.log).

`MAILSTRONAUT_REPLY_GUI=1` extends the existing private
`experiments/p04-inbox/gui.sh` path called by the encrypted-keyring harness.
Supply prebuilt `MAILSTRONAUT_INBOX_GUI_BINARY`, `MAILSTRONAUT_IMAP_FIXTURE` and
`MAILSTRONAUT_CREDENTIAL_FIXTURE`, then run:

```sh
nix develop .#gui-test --command bash experiments/p02-accounts/credentials.sh
```

The final run passed in X11/Xvfb at 100% with a private bus/keyring/profile and
localhost TLS IMAP. It downloaded one canonical synthetic original, invoked each
native reply button and checked all three stored drafts. An edit followed by an
immediate switch to reply-all survived. Normal close and offline/light-mode restart
then opened the saved reply in the native editor; selection/copy recovered its
last edit and original quote. Outbox remained empty, no STORE/EXPUNGE was sent,
and server read flags were unchanged. Private credential and unavailable-service
checks passed afterward. No user account, bus, keyring or real recipient was used.

- [Native GUI log](artifacts/p07-reply/native-gui.log)
- [Reply](artifacts/p07-reply/native-reply.png)
- [Explicit text forward](artifacts/p07-reply/native-text-forward.png)
- [Reopened offline reply](artifacts/p07-reply/native-offline-reply.png)

This does not accept live-provider reply delivery, attachments, signatures,
keyboard/screen-reader or Wayland behavior, untrusted HTML, current-source release
package, CI containers or Flatpak. Earlier P05/P07 package evidence covers only
its recorded commits. No real mail was sent, and full P07 checklist rows stay open.

The workspace now directly declares its existing configuration/Chrono dependencies;
no third-party version or feature was added. Cargo.lock's workspace entry changed,
so the vendor staging hash is updated to
`sha256-EbgPgrRKV8i6lLA2ZkTaxgtg5AouuN4J6mFDf8Abitw=`.
The standard `nix build .#mailstronaut.cargoDeps --no-link --no-update-lock-file`
passed and returned
`/nix/store/jk00sgjfkrpyimf5inn0rxdi677kkmw0-mailstronaut-0.1.0-vendor`.
Local rehydration used the same fsck-verified exact Snix cache as P05; no mirror
or pin change is part of the flake. This vendor check is not a release package gate.
