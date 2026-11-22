# P10 — Home Manager activation without stubs

`ci/home-manager-activation.sh` builds the optional `programs.mailstronaut` module through a real
Home Manager configuration. `nix/home-manager-activation.nix` pins Home Manager by source hash
(revision `095fef2f`), so `flake.lock` is unchanged. The script runs each generated activation
script in a throwaway home directory under `target/` on the CI server. Dotfiles and the host's
configuration are not touched.

Passing on the CI server, in the `gui-test` development shell, with each check asserting the
evaluated configuration rather than matching text:

- Immutable: `~/.config/mailstronaut/config.nix` is a link into the store, with the managed header.
- Mutable, first activation: the file is a writable copy owned by the application.
- Mutable, repeated activation with a changed declaration: the declared `appearance` wins, and a
  UI edit to an undeclared key (`sync.pollSeconds`) survives.
- Removing a declaration stops enforcing it but keeps the current value and the UI edit.
- An explicit empty account list replaces the declared accounts, because whole lists replace.
- An invalid declaration (`appearance = 42`) fails the activation, and the configuration file is
  byte-identical afterwards.
- An immutable activation does not replace a user-owned regular file at the configuration path.
- A mutable activation does not silently overwrite a user-owned file that the application did not
  create.

## Local NixOS recheck

The local x86_64 NixOS run on application source `722472b` passed all eight scenarios
above and two additional ones:

- A competing writer holds the actual configuration lock. Activation fails with the
  expected conflict, the source checksum stays unchanged, and activation succeeds
  once the lock is released.
- The installed client's `config export` output becomes the `settings` expression of a
  new Home Manager generation. A fresh disposable home receives the exported appearance,
  900-second interval and declared account in a writable native file.

The fixture uses a separate home, profile, XDG runtime directory and bus address.
Home Manager installed the real package and skipped desktop-service activation. Use the
[maintenance guide](../MAINTENANCE.md) for backup and redeclaration. This verifies the CLI
export round trip. The GUI edit/portal export workflow and autostart on the live desktop
have separate acceptance requirements.

The later application revision `df73925` passed local x86_64 `nix flake check`,
`nix build`, `nix run -- --version`, and installed native configuration validation.
Before compilation the run compared the workspace and vendored lockfiles. Its
receipt is `nix-gate-exit=0` in `target/local-checks/latest-packages.log`.
The release gate exercised the flake's Home Manager module check. The disposable
generation scenarios above used `722472b`.

## Opt-in native login startup

A local disposable activation on 2026-11-22 enabled
`programs.mailstronaut.startOnLogin`. The pinned Home Manager installed the
generated user unit and its `graphical-session.target.wants` link. The service
invokes the packaged binary with `--background --config` and the exact native
file path, and is conditional on `WAYLAND_DISPLAY`. `systemd-analyze --user
verify` accepted the generated unit. A subsequent generation removed both
service links while keeping `background.enabled = true` in the native file.
The activation used the known local Nix package from `395c52b`; the fixture
isolated the user service manager from the actual desktop. The upgraded
application also passed the private Wayland/Dovecot walkthrough with the
service's windowless launch arguments, background sync, launcher reopening
and explicit Quit. Captures are under `target/p02.6lqcBa/background-gui`.
Live login/logout, suspend and the Flatpak Background portal remain open.
