# Mailstronaut data and recovery

## Back up a profile

Quit Mailstronaut with **Ctrl+Q** so its background process stops. If you run the
Home Manager user service, stop that service before making or restoring a copy.
Keep all Mailstronaut processes stopped until the operation finishes.

Back up the complete data directory together with your native configuration.
The database, its SQLite sidecars, original mail and attachment blobs, drafts,
outbox, reminders and deadline corrections must come from the same snapshot.
A backup with just `mail.sqlite3` may lack both mail bodies and committed
transactions. Store the backup on encrypted storage with access limited to you.

| Installation | Data | Default configuration |
| --- | --- | --- |
| Nix or native | `$XDG_DATA_HOME/mailstronaut` (defaults to `~/.local/share/mailstronaut`) | `$XDG_CONFIG_HOME/mailstronaut/config.nix` (defaults to `~/.config/mailstronaut/config.nix`) |
| Flatpak | `~/.var/app/io.github.eduardz1.Mailstronaut/data/mailstronaut` | `~/.var/app/io.github.eduardz1.Mailstronaut/config/mailstronaut/config.nix` |

If you use `--config`, back up that source. For Home Manager, also save the
declarations that generate the configuration. Copy the effective file too if
you want to retain changes made through **Settings**. **Export to Nix** gives
you a reviewable declaration to carry those changes back into Home Manager.
[Configuration and ownership](CONFIGURATION.md)

Credentials live in the desktop keyring. A restored profile may ask you to sign
in or enter an app-specific password again. Optional model files live at their
declared paths and can be reinstalled independently.

## Upgrade or roll back

Take a full profile backup before upgrading. When a database schema changes,
Mailstronaut writes a consistent copy of the previous SQLite database to the
profile's `backups/` directory before migrating. That copy is useful for
diagnosis and is only one part of a complete profile backup.

To roll back, stop Mailstronaut, move the upgraded profile aside, and restore the
full pre-upgrade data and configuration alongside the old package. Keep each
database with the original blobs and sidecars from its own snapshot. The older
client refuses a database with a newer schema. Switching the Nix package or
Home Manager generation alone leaves the on-disk database at its current
schema.

Before syncing a restored outbox, check any mail previously marked as having
an unknown delivery outcome against the provider's Sent folder. Review the
original delivery status before choosing to send again.
[Delivery and retry behavior](DELIVERY.md)

## Cache, accounts and Thunderbird mail

Disabling or removing an account declaration stops its connection. Its local
mail, drafts and reminders can still exist on disk. Derived search indexes can be
rebuilt from the profile. The application does not yet expose a complete account
purge workflow or guarantee physical erasure of deleted bytes.

Mail that already resides on an IMAP server can be downloaded again by adding
that account. Mailstronaut has no direct importer for Thunderbird Local Folders
or its profile database. Keep a copy of that Thunderbird profile. If you choose
to move local-only messages through a provider, copy a small sample to a dedicated
IMAP folder from Thunderbird first. Verify the imported originals and
attachments in Mailstronaut before copying the rest. This route uploads those
messages to the provider. Server quota and folder retention still apply.

Mailstronaut keeps cached mail on the device and does not encrypt its mailbox
database or blob directory. Use an encrypted disk for the profile and its backup.
The reader blocks automatic remote resources. Encrypted messages without readable
content stay out of local analysis. The background cache and local search only
cover mail the device has actually fetched.
[Rendering and privacy](RENDERING.md) · [Search](SEARCH.md)
