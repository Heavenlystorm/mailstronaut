# Native account configuration and credentials (2026-10-29)

This slice implements native Nix startup/reload and foreground password setup. It
is not an IMAP/SMTP client or completed P02/P05 acceptance. No real account,
password, mail, Home Manager activation or host keyring was used.

## Implementation

- The application starts on Accounts. `--config FILE` selects an explicit native
  source; otherwise it uses the XDG config location. A missing source opens setup;
  a broken symlink or invalid source shows a redacted error and starts no accounts.
- File reads run off the UI thread, follow managed symlinks and retain the 256 KiB
  limit. Nonregular files are rejected, and the file open is nonblocking so a FIFO
  swapped in after metadata cannot stall that open. Evaluation uses the existing
  limited same-binary pure Snix helper. No host Nix or hidden configuration format.
- The complete validated declaration is adopted as an immutable snapshot. Failed
  or missing reloads retain the last valid snapshot; successful adoption advances
  its revision. Request generations/cancellation prevent stale load results from
  replacing a newer declaration. Existing files are treated as user-managed and
  never rewritten. Removing accounts does not invoke credential deletion.
- Account rows expose password setup, including an independently configured SMTP
  authentication reference. OAuth rows explicitly report the pending browser
  authorization flow. Static preferences use the native COSMIC theme; effective
  dark/light selection is forwarded to Servo's native theme method.
- The new `mailstronaut-mail` crate owns a small standard oo7 0.6.0 adapter.
  Construction/unlock are explicit foreground actions: oo7 may prompt while
  obtaining a portal secret or creating a missing collection. Cached reads/writes
  check lock state and never request unlock. No plaintext fallback is implemented.
- Credential identity includes application, stable account ID, secret reference
  and password/refresh-token purpose. Ambiguous matches are rejected. Reads,
  rotation and exact deletion have 15-second operation budgets; foreground
  connection/unlock has 120 seconds. Values use secrecy 0.10.3, are limited to
  64 KiB and are redacted from event/error Debug output. No server diagnostics or
  secret values are retained in errors. The masked input clears on submission,
  page exit and configuration adoption.
- Account setup and native Text mode no longer initialize Servo. The synthetic
  renderer is created lazily on the first Preview visit, then retained under the
  existing process-global lifecycle constraint. Arbitrary mail HTML stays disabled.

Upstream APIs: [oo7 Keyring](https://docs.rs/oo7/0.6.0/oo7/enum.Keyring.html),
[secrecy SecretString](https://docs.rs/secrecy/0.10.3/secrecy/type.SecretString.html).
Native Secret Service encryption/unlock policy belongs to the desktop provider;
the application uses its standard API. The tested disposable provider was GNOME
Keyring 50.0 with a nonempty synthetic fixture key.

## Pinned package and behavior checks

The coordinated service `mailstronaut-accounts-gate-r3`, invocation
`fd84a1f8d717440aaf5208519b40ae30`, ran:

```sh
nix flake check --no-update-lock-file --print-build-logs --cores 16 --max-jobs 1
```

It exited **0** on x86_64-linux, with stable Rust 1.99.0, LLVM, thin LTO and one
codegen unit. Strict release Clippy passed before the large executable link.
All **39 workspace tests**, **seven isolated renderer child invocations**, and
**two private credential acceptance tests** passed. aarch64-linux was omitted.

The real-helper source tests cover missing startup, managed symlinks, nonregular
and oversized files, UTF-8 rejection, cancellation, invalid/schema-version reload,
retaining revision 1, repair/adoption of revision 2 and unchanged source bytes.
An application-event regression checks password Debug redaction.

The private credential harness runs its own D-Bus daemon without service
activation directories and its own GNOME Keyring under private project XDG
paths. It verifies persistence through a new connection, rotation, account and
purpose isolation, exact deletion, duplicate rejection and locked background
read/write/delete failure. After stopping the service it verifies unavailable
storage fails closed and persisted data hashes stay unchanged. Synthetic secret
markers are absent from the persisted keyring bytes. The package's `postCheck`
now runs this harness; the two tests remain ignored in ordinary unspecialized
Cargo test runs to prevent accidental use of the user's session bus.

The new vendor hash is
`sha256-BtdcAm2vHeaBTkXOZlU/imllvVUh3QpiAuVUGaaNm+A=`.
The dependency archive completed once in the first coordinated run; that run's
superseded application link was stopped after the archive was registered. The
final source reused `/nix/store/jv8j8giqx3rzkn2w3zshcyky65k3y2rj-mailstronaut-deps-0.1.0`.
An intermediate run rejected one documentation lint before linking; it is not a
passing gate. No speedup measurement is claimed.

Validated package:
`/nix/store/nxi38afxdfh3xhbhm7hrsz9pa2hfwqzl-mailstronaut-0.1.0`.
Its installed CLI/configuration smoke passed with an empty environment. All 25
Rust/manifests/lock files match the built source snapshot. Its 35-path runtime
closure contains none of the private GNOME Keyring, libsecret CLI, Zenity,
pyatspi or dependency-archive test inputs.

## Installed GUI checks

```sh
nix develop .#gui-test --command env \
  MAILSTRONAUT_ACCOUNT_GUI_BINARY=/nix/store/nxi38afxdfh3xhbhm7hrsz9pa2hfwqzl-mailstronaut-0.1.0/bin/mailstronaut \
  bash experiments/p02-accounts/credentials.sh
```

The private X11 walkthrough passed missing-source startup, configured-account
startup, explicit keyring unlock, masked entry, Ctrl+A/C clipboard protection,
password save and independent readback through the standard `secret-tool`.
The managed source remained byte-identical after save; the synthetic password
marker was absent from application logs. Captures show invalid reload retaining
revision 1 and the repaired source adopting revision 2.

The existing native Text regression also passed at initial X11 100/150/200%:
21 full-body clipboard checks, keyboard/mouse selection, scroll, theme, resize,
keyboard reentry and returning to Servo. The AT-SPI observer still sees an empty
application frame while its GTK positive control publishes its label/button.
All three accessibility results remain **blocked**, not passing.

Artifacts: [summary](artifacts/p02-accounts/summary.json),
[release log](artifacts/p02-accounts/release-gate.log),
[GUI log](artifacts/p02-accounts/gui.log),
[source hashes](artifacts/p02-accounts/source-check.json),
[installed smoke](artifacts/p02-accounts/installed-smoke.log).

## Remaining acceptance

No live sign-in, IMAP/SMTP operation, OAuth callback, locked-desktop prompt,
Flatpak Secret portal, actual Home Manager link/activation, Wayland account-form
acceptance, remote CI, or complete P01/P02/P05/distribution claim is made. The
configuration schema still needs its full validation/ownership acceptance; no
GUI rewriting/adoption of an existing Nix source was added. The priority,
background, notification and synchronization declarations are not operational
mail features yet. Mail data/storage, threading and transport integration follow.
