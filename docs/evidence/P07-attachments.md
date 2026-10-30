# P07 — Owned compose attachments

P07 remains incomplete. This slice adds local file attachments to the existing
plain-text composer; untrusted HTML rendering, received-attachment export,
signatures, live provider acceptance and distribution acceptance remain separate.
All fixtures use synthetic files, accounts and local protocol servers.

## Behavior

- Native libcosmic Attach/Remove controls use ASHPD's standard FileChooser portal.
  The selected draft is fenced through asynchronous selection and storage replies;
  controls from an old draft cannot operate on its replacement. Editing/sending
  waits while selection/import is pending. File selection does not unlock a keyring
  or initiate network activity.
- Copying happens on a blocking task, outside the UI and SQLite owner. Only local
  regular files with valid UTF-8 basenames are accepted. Imports are bounded to 16
  attachments, 255-byte basenames and 5 MiB total decoded bytes. FIFO/device reads
  cannot block the copy worker. Source access loss, size/change failures and portal
  errors are visible; a failed multi-file import does not attach a partial subset.
- Once imported, bytes belong to the private profile's content-addressed blob
  store. No source paths or portal grants are needed for draft restart/recovery.
  Schema 7 stores revisioned draft-local references; upgrades retain existing
  drafts and create the existing schema backup. Blob writes and collection share
  SQLite's IMMEDIATE transaction coordination. Attachment references protect
  snapshots from orphan collection, including independent account copies.
- Lettre builds multipart/mixed with a plain body and binary attachment parts;
  every file uses application/octet-stream. It owns MIME encoding and Unicode
  filenames. The full frozen original remains bounded to 8 MiB. Bcc is private
  envelope state, never a transmitted header. Before queueing, every owned blob is
  read with the existing regular-file/hash checks. Missing or damaged files prevent
  queueing and keep the draft editable, allowing explicit removal/replacement.
- Changing accounts creates an independent editable copy retaining the snapshots.
  Definite-send rejection recovery retains them too. Retry uses the original frozen
  MIME and Message-ID. No original file is reopened during submission, and no
  automatic retry or real email send is introduced.

## Development and native validation

`nix develop --command bash ci/check.sh` passed on the settled implementation:
formatting, Nix formatting, Actionlint, ShellCheck, strict workspace Clippy and
143 workspace tests, plus seven isolated renderer child invocations. The five new
store integration tests cover schema-6 upgrade/backup, source removal/restart/GC,
freeze/retry/rejection recovery, cross-account independent copies, stale/foreign
edits, missing/corrupt files and atomic count/size/name failures. Existing synthetic
TLS SMTP tests now submit a binary attachment and compare captured DATA with the
exact frozen original. They exercise mandatory STARTTLS and implicit TLS,
credential withholding, Bcc privacy and ambiguous acknowledgments.
[Development excerpt](artifacts/p07-attachments/development-check-excerpt.log).

The private FileChooser fixture passed binary grants, cancellation, remote URI
rejection, missing source, count and aggregate-size failures. Its optional native
walkthrough passed with the final development binary in `nix develop .#gui-test`:
Attach, Remove, attach again, graceful close, deletion of the selected source,
restart, opening the saved draft and removing the retained snapshot.
[Receipt](artifacts/p07-attachments/native-portal.log),
[attached](artifacts/p07-attachments/attached.png),
[recovered after source deletion](artifacts/p07-attachments/recovered.png),
[removed after recovery](artifacts/p07-attachments/recovered-removed.png).
Full private fixtures: `target/p07-attachments/portal.9EcXLApY`.

The harness uses a private D-Bus server and Xvfb, synthetic account endpoints and
no SMTP connection or keyring unlock. It proves the native controls against the
portal protocol fixture, not a real COSMIC file dialog or a Flatpak document grant.
No provider, accessibility or distribution acceptance is claimed.

The existing private keyring/native compose regression also passed with the new
Attach row: editable fields, autosave, close-before-debounce flush, restart,
explicit synthetic TLS SMTP send and exact captured frozen DATA/Message-ID.
[Receipt](artifacts/p07-attachments/native-compose-regression.log). This used the
prebuilt final development binary and fixture executables in `nix develop .#gui-test`;
artifacts are in `target/p02-accounts/keyring.3QV7iqbL/compose-gui`.

## Release/package gate

The coordinated `nix flake check --no-update-lock-file --print-build-logs` gate
passed on x86_64-linux for exact implementation commit
`d3572fe7e732870a3944674a1b68a242c3535f8a`. The runner used all 16 CPUs and one
Nix job, retaining stable LLVM, thin LTO and one codegen unit. It reused the
existing dependency artifact `/nix/store/j2k9b80y20cfcldzd9qc83v4xy90ysc3-mailstronaut-deps-0.1.0`.
Release Clippy, 143 workspace tests, seven isolated renderer children and six
private portal/credential/OAuth fixture invocations passed. The service
`mailstronaut-attachments-nix-gate.service`, invocation
`d2fac5e547c04f92bff873cf1fadfa36`, exited 0; its fresh receipt is 0 and it is
inactive/dead with `ExecMainStatus=0`. The complete log ends `all checks passed!`.
`aarch64-linux` was omitted. [Receipt](artifacts/p07-attachments/release-gate.status),
[revision](artifacts/p07-attachments/release-gate-revision.txt),
[excerpt](artifacts/p07-attachments/release-gate-excerpt.log).
Full log: `target/p01-package-gate-service.log`.

The subsequent cached `nix build --no-update-lock-file` and installed CLI/config
smoke passed. Package:
`/nix/store/fwgymqzrygvyd3zfxgwk21awpssllr3r-mailstronaut-0.1.0`.
[Installed smoke](artifacts/p07-attachments/installed-smoke.log); full smoke fixture:
`target/p01-installed-smoke.Y88TsC`. This covers same-binary configuration evaluation
without an ambient PATH, safe initialization permissions/no-overwrite and schema
rejection. It adds no release GUI, hosted CI, real-provider or Flatpak acceptance.
