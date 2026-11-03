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

Not established: concurrent configuration sources, the export and import documentation, and
activation on NixOS. Those remain open, so the P10 row stays unchecked.
