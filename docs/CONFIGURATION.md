# Native Nix configuration and account authentication

This is the **target contract** for P02/P05/P08/P10. The retained backend implements
bounded native evaluation, last-valid source state, encrypted credentials and
public-client OAuth/refresh. GPUI Settings now connects declared accounts to
foreground password storage and explicit bounded TLS inbox sync; native last-valid
reload and offline reading passed the private synthetic workflow in
[current account evidence](evidence/P02-gpui-accounts.md). Native export and the optional
HM module, basic editable preferences and native account declarations are implemented;
their acceptance scope is recorded in PLAN.md. Full Home Manager activation
and real-provider/portal acceptance remain open. Canonical user configuration
is `$XDG_CONFIG_HOME/mailstronaut/config.nix`.
Flatpak uses its usual application-specific XDG config directory. A `.nix` file is
the actual input consumed by the Rust application in both distributions.

## Native evaluation, with no translation module

Use `snix_serde::from_str::<Config>(&source)` to evaluate a self-contained pure Nix
expression and deserialize directly into Rust. Its pinned implementation constructs
`EvaluationBuilder::new_pure().mode(EvalMode::Strict)`. This is an in-memory typed
value conversion, not a second user configuration format. Do not invoke `nix eval`,
install Nix into Flatpak, generate TOML/JSON configuration files, mirror all Rust
fields in a separate HM schema, or implement a partial home-grown Nix evaluator.
An optional small HM module may manage native-file installation and mutability.

The supported runtime language is pure data, `let`, attribute references, lists,
conditionals and pure builtins provided by the pinned evaluator. It is not a
Nixpkgs/Home Manager evaluation environment: no `pkgs`, `lib`, `config`, `<nixpkgs>`,
flakes, imports, environment reads, fetching, derivations, shell execution, or Nix
store daemon. Home Manager can compose values at activation and write a final
self-contained Nix expression, or simply link a self-contained source file. Path
settings are strings so pure evaluation does not need filesystem path coercion.
Test every promised language feature; document unsupported builtins explicitly.

Configuration is trusted local user code but must not freeze the UI. Bound input
to 256 KiB, deserialize with unknown-field rejection, validate the complete value
before applying, and show field-specific errors without secrets. Perform evaluation
in a short-lived same-binary helper (`--internal-evaluate-config`), using the pure
evaluator, stdin/stdout with a bounded typed internal result, a two-second timeout
and a 1 GiB address-space limit (including the monolithic executable's mappings).
Target helper resident memory <=128 MiB. Spawn by absolute executable path, without a
shell; kill/reap on timeout. Use the existing safe `rustix` resource-limit API in
the helper. Internal IPC may use Serde JSON; no user JSON config is created or
maintained. This helper is not a daemon, network service, or external Nix dependency.
P02 must measure the real evaluator memory/startup cost and adjust this documented
limit if necessary with evidence. Reject recursive/nonterminating examples safely.

Snix's bridge currently enables the evaluator's default features, including
`impure`, `arbitrary`, and `nix_tests`; setting `default-features = false` on a
separate direct dependency does not undo that transitive choice. Pure construction
still governs evaluation permissions. Audit the closure and propose a small upstream
feature fix if needed; do not accidentally include snix-cli, store, glue or a daemon.

Missing configuration opens a welcome/setup screen without starting account
connections. `mailstronaut config init` writes a native Nix template if none exists;
`mailstronaut config check [--path FILE]` evaluates/validates and exits without
networking or credentials. `--config FILE` overrides only the explicit file path.
An invalid startup file opens a diagnostic screen and starts no network work. On
an invalid reload, retain the last valid running configuration and display the
failure; never half-apply accounts or delete runtime data. Explicit reload first;
add a file watcher only if needed, watching the parent directory for atomic symlink
replacement. Increment config revision on success and cancel stale analysis/auth.

## Example canonical schema v1

Save the following as `mailstronaut.nix`. All values are examples, and empty
accounts are valid. Unknown fields and duplicate/empty account IDs are errors.

```nix
let
  personalAddress = "person@gmail.com";
in
{
  schemaVersion = 1;
  managed = true;
  locale = "en-GB";
  timeZone = "Europe/Paris";
  appearance = "system"; # system | light | dark; always native themes

  background = {
    enabled = true;
    autostart = false;
  };
  notifications = {
    enabled = true;
    showPreview = false;
  };
  privacy = {
    remoteContent = "ask"; # v1: ask | never; neither loads on message open
  };
  sync = {
    automatic = true;
    # Optional age limit in days. Omit it, or write null, to fetch each folder's complete
    # history; a number keeps the older cap for mail that arrived within that many days.
    initialDays = null;
    pollSeconds = 300;
    maxBodyBytes = 2097152;
  };
  priority = {
    enabled = true;
    modelDirectory = null; # null: rules only, no network/download
    modelRevision = null; # required manifest identity when a model is supplied
    languages = [ "en" "fr" "it" ];
    defaultTtlHours = 72;
    maxTtlHours = 168;
  };
  reminders = {
    morning = "09:00";
    evening = "18:00";
  };

  accounts = [
    {
      id = "personal-google";
      provider = "google";
      address = personalAddress;
      displayName = "Personal";
      userName = personalAddress;
      aliases = [ ];
      signature = "Person Example\nDepartment of Examples";
      auth = {
        kind = "oauth2";
        provider = "google";
        clientId = "YOUR-REGISTERED-DESKTOP-CLIENT-ID";
        secretRef = "personal-google-oauth";
      };
      # google preset supplies imap.gmail.com:993/TLS and
      # smtp.gmail.com:587/required STARTTLS, and Gmail folder behavior.
    }
    {
      id = "personal-icloud";
      provider = "icloud";
      address = "person@icloud.com";
      displayName = "iCloud";
      userName = "person"; # Apple also supports full address if needed
      smtpUserName = "person@icloud.com";
      auth = { kind = "password"; secretRef = "personal-icloud-app-password"; };
    }
    {
      id = "university";
      provider = "manual";
      address = "person@univ-pau.fr";
      displayName = "University";
      userName = "login@univ-pau.fr";
      imap = { host = "partage.univ-pau.fr"; port = 993; security = "tls"; };
      smtp = { host = "partage.univ-pau.fr"; port = 465; security = "tls"; };
      auth = { kind = "password"; secretRef = "university-password"; };
      folders = {
        inbox = "INBOX";
        # Optional sent/drafts/trash/archive override server SPECIAL-USE discovery.
      };
    }
  ];
}
```

Define Rust structs with `#[serde(rename_all = "camelCase", deny_unknown_fields)]`
and explicit tagged enums. Fields omitted above take these defaults: aliases `[]`, no signature,
smtpUserName defaults to userName except for iCloud, where it defaults to the address; notifications enabled with private previews,
background disabled/autostart false, theme system, privacy ask, complete history (no age
limit) for sync, five-minute polling, 2 MiB analysis-body cap, model absent/rules only, 72-hour normal
TTL and 168-hour cap. Absence of the entire priority section defaults to disabled
until the setup screen explains its local behavior. Do not enable model downloads
through a default. `schemaVersion` is mandatory; reject future versions with a
clear supported range. Do not rewrite a managed file during migrations.

For an institutional account whose server uses a short login, edit the account's
incoming login in Settings when the source is app-owned, or set `userName`
inside that account's entry in the `accounts` list, for example
`userName = "eocchipi";`. If SMTP uses a different login, set `smtpUserName`
in the same entry; otherwise omit it to use the provider default (the address for
iCloud, `userName` for other providers). Reload the
configuration in Settings after editing. For a Home Manager managed declaration,
edit the declaration and activate Home Manager before reloading.

Manual account requirements: host, port, `security = "tls" | "starttls"`, incoming
and outgoing login names, auth method, optional `smtpAuth` override (same tagged
schema as auth), optional `tls.caFiles = [ "/path/to/institution.pem" ]`, folder
mapping and aliases. No unauthenticated plaintext mode, arbitrary shell commands,
inline passwords/tokens, “accept invalid TLS” switch or custom auth scripts.
Defaults remain secure even if ports are unusual.

Custom OAuth2 `auth` extends the example as follows; enforce HTTPS endpoints and a
known allowed loopback redirect policy. Do not implement discovery that trusts
arbitrary messages or follows token endpoint redirects.

```nix
auth = {
  kind = "oauth2";
  provider = "custom"; # or microsoft preset
  clientId = "REGISTERED-PUBLIC-CLIENT-ID";
  authorizationUrl = "https://identity.example.edu/oauth2/authorize";
  tokenUrl = "https://identity.example.edu/oauth2/token";
  scopes = [ "IMAP_SCOPE_FROM_THE_PROVIDER" "SMTP_SCOPE_FROM_THE_PROVIDER" ];
  sasl = "xoauth2"; # xoauth2 | oauthbearer; respect server capabilities
  secretRef = "institution-oauth";
};
```

## Home Manager directly installs native configuration

Native Nix install, without a Mailstronaut HM module:

```nix
{ inputs, pkgs, ... }:
{
  home.packages = [ inputs.mailstronaut.packages.${pkgs.stdenv.hostPlatform.system}.default ];
  xdg.configFile."mailstronaut/config.nix".source = ./mailstronaut.nix;
}
```

Home Manager may instead use `lib.generators.toPretty { } nativeConfig` in
`xdg.configFile."mailstronaut/config.nix".text` when values must be composed from
the user's other Nix settings. That still writes the app's **native Nix** format.
No duplicate per-field schema or Nix→TOML layer is part of this project.
The account list is the authoritative declaration; OAuth refresh tokens, passwords,
sync cursors, drafts and window placement are not declarative config.

### Optional module and mutable settings

Keep the direct-file recipe above. The flake also exports a small optional module as
`homeManagerModules.default`, with `programs.mailstronaut.enable`, `package`,
`settings` and `mutableSettings`. `mailstronaut config reconcile --declared FILE`
performs the mutable merge at activation, and `mailstronaut config export` prints a
secret-free snapshot to adopt. Settings → Export to Nix (Ctrl+Shift+E) uses a native
read-only Kit dialog, Clipboard and the platform's portal save chooser. Save creates
a new private file; existing files and links are preserved. The snapshot contains
effective defaults, accounts and keyring references, with no credentials or mail state.
`settings` contains the same native data accepted
by the application, rather than an independently maintained option per Rust field.
Use `lib.generators.toPretty` for native Nix data, with the application's existing
evaluator/schema as the final validation boundary. Ownership field `managed` is
set by the chosen deployment mode rather than duplicated as a preference.

```nix
{
  imports = [ inputs.mailstronaut.homeManagerModules.default ];
  programs.mailstronaut = {
    enable = true;
    mutableSettings = true;
    settings = {
      appearance = "dark";
      timeZone = "Europe/Paris";
      # accounts = [ ... ]; # Same schema as config.nix, including secretRef.
    };
  };
}
```

Exports use the pinned Snix serializer. Raw reconciliation preserves explicit keys
through its pure `builtins.toJSON` inside the existing limited helper: the pinned
bridge's generic string visitor returns display quotes. This small workaround uses
the correct typed-string deserialization path; its JSON is transient internal data,
with the same input/output/time/memory bounds. It creates no user JSON file and
requires no host Nix executable or dependency fork.

`mutableSettings = false` is the default: install a managed native `.nix` file,
and keep persisted preferences read-only in Settings. `mutableSettings = true`
opts into an app-owned writable regular `config.nix`, mode `0600`, with
`managed = false`. It is not an out-of-store link into the user's dotfiles and
does not introduce a TOML file or a hidden preference overlay.

### Editing preferences in the application

Settings exposes appearance, automatic synchronization and its polling interval,
reminder notifications and private previews, timezone, and morning/evening defaults.
It reuses Kit's settings fields, segmented time editors and searchable IANA timezone
picker. Changes are staged until **Save preferences**; **Discard changes** restores
the loaded values. The actions stay visible while filtering Settings. A saved
candidate reloads through the same validated account/runtime adoption path as an
explicit reload. The toolbar theme toggle remains a temporary preview.

The source path, ownership and last reload are visible alongside persistent errors.
Only an app-initialized or mutable-HM regular writable source with `managed = false`
is editable. Managed sources, read-only files, other links and unadopted expressions
stay read-only; encrypted credential actions remain available separately. Saving
serializes native data from the raw explicit keys, changing only the staged fields
before defaults. It can replace expressions/comments in an app-owned file, so keep
handwritten expressions as read-only sources or in the HM declaration.

The application and activation share the same lock and atomic writer. A save
rejects a different effective source than the loaded snapshot, including a newer
HM activation. Reload before retrying; errors preserve the source and last valid
runtime. Export includes saved effective values, so copy it into HM to retain UI
edits to declared keys across the next switch.

Use familiar activation semantics: explicitly declared attributes take precedence
over the current writable configuration on each HM activation; other UI-set values
survive. Attribute sets merge recursively; scalars, nulls and whole lists replace.
Do not expand missing declarations to schema defaults before reconciliation, as
that would reset undeclared preferences. Apply and validate defaults only after
merging. In particular, `accounts = [ ];` explicitly disables all connections,
whereas omitting `accounts` preserves the current mutable list. Removing a
declaration relinquishes enforcement; it does not erase the current value or
delete mailbox data. Explain in Settings that UI edits to declared values can be
overwritten at the next switch; export lets the user adopt those edits in HM.

The user's Herdr example follows static-wins mutable activation. Its pinned
[HM module](https://github.com/nix-community/home-manager/blob/095fef2f265c3c0da4fe315efba8f8a04495ab34/modules/programs/herdr.nix)
uses HM's standard merger. The pinned
[helper](https://github.com/nix-community/home-manager/blob/095fef2f265c3c0da4fe315efba8f8a04495ab34/modules/lib/generators.nix)
supports JSON/YAML/TOML/CBOR, not native Nix output. Do not assume a universal
`mutableSettings` facility or route our native Nix through a persisted foreign
format to use it. Reuse the existing Snix evaluation/serialization boundary for
the small reconciliation operation; do not add a parser or general configuration
framework. Any intermediate typed data is internal IPC, not a second user file.

Activation and UI save must coordinate writes to the same app-owned path. Validate
the complete candidate, retain the last valid runtime state on failure, and use a
temporary sibling plus atomic replacement. Reject stale/concurrent edits and
unexpected links instead of clobbering them. Changing from immutable to mutable
may replace only the module's known managed link after checking its identity;
never write into its Nix-store target. Changing back must preserve changed files
through explicit collision/backup handling rather than silently discard UI edits.
Test these cases in a disposable HOME, not by activating the user's configuration.

`config init` and mutable activation mark app-owned regular sources. Reconciliation
refuses an unmarked existing expression, a read-only file, or an unrelated link;
export and back up an authored expression before creating an app-owned source.
Only a marked module-generated store link may be replaced in place. A persistent
sibling `.config.nix.lock` serializes cooperating writers, and the source bytes and
file identity are rechecked before atomic replacement. Uncooperative external
editors should use that lock for atomic cooperation; detected edits are rejected.
The module's immutable path uses ordinary HM collision handling, without `force`:
returning to immutable mode can require backing up an existing mutable file.

For an already installed Flatpak, Home Manager can link the **same native file**:

```nix
home.file.".var/app/io.github.eduardz1.Mailstronaut/config/mailstronaut/config.nix".source =
  ./mailstronaut.nix;
```

Flatpak cannot normally traverse a symlink into `/nix/store`. Provide an explicitly
documented Nix-managed Flatpak override granting `/nix/store:ro` if this direct-link
mode is chosen, or have HM activation copy the same native `.nix` bytes atomically
to a regular file in the sandbox's config directory. Prefer a regular native file
copy for a least-privilege Flatpak setup; it is **not format conversion**. Do not add
host/home filesystem permissions to every distributed Flatpak for this edge case.
ModelDirectory and CA paths also need to be visible in the sandbox: install/import
those files there or use a document portal. Do not promise access to host secret
file paths through Flatpak.

The P08 target for native login startup uses standard Home Manager service
composition. The current binary still rejects `--background`; enable this service
only after PLAN.md's background-lifetime row is accepted:

```nix
{ inputs, pkgs, ... }:
let
  mailstronaut = inputs.mailstronaut.packages.${pkgs.stdenv.hostPlatform.system}.default;
in
{
  systemd.user.services.mailstronaut = {
    Unit = {
      Description = "Mailstronaut background mail and reminders";
      After = [ "graphical-session.target" ];
      PartOf = [ "graphical-session.target" ];
      ConditionEnvironment = "DBUS_SESSION_BUS_ADDRESS";
    };
    Service = {
      ExecStart = "${mailstronaut}/bin/mailstronaut --background";
      Restart = "on-failure";
      RestartSec = 5;
    };
    Install.WantedBy = [ "graphical-session.target" ];
  };
}
```

The service is installed only when the user's configuration chooses it. Desktop
environment variables must be imported into the user manager; document/debug that
with native Wayland, do not set fake WAYLAND_DISPLAY values. Native service-managed startup
and portal autostart are mutually exclusive to avoid duplicate launches. The
`autostart` field expresses intent for app-managed portal startup; in HM-managed
native service mode show “managed by Home Manager” and do not write an autostart
entry or toggle systemd from the app. Stopping/disabling/restarting that user service
is a Home Manager/user decision. Explicit Quit exits successfully and is not restarted.

## GPUI Settings and source ownership

Use GPUI Kit's `Settings`/`SettingPage`/`SettingGroup`/`SettingItem` hierarchy and
standard `SettingField` controls, retaining built-in search and disabled states.
See [INTERFACE.md](INTERFACE.md). Display the effective source path, ownership,
last reload result and whether an edit is saved. Do not enable controls for features
that have no working backend. Masked credential entry and browser sign-in are
separate runtime actions, not serializable preference fields.

`managed = true` or a read-only/Nix-store-backed config prevents the GUI from
rewriting it. Account rows and static preferences are displayed with their source;
Connect/Reauthenticate remains available because credentials are runtime secrets.
On a missing source, Configuration offers Create native configuration; it shares
the CLI's locked, no-clobber initialization and creates no account or credential.
The Add account page uses standard GPUI Kit form fields for Google, iCloud and
manual connections. Save appends a validated declaration to an app-owned source,
preserving explicit keys and omissions in existing accounts, then opens that
account's encrypted credential controls. Manual connections support distinct
incoming/outgoing logins and passwords, additional PEM trust roots, aliases and
Sent/Archive/Trash overrides. Google and Microsoft require the user's registered
public desktop client; custom OAuth requires explicit HTTPS endpoints/scopes.
Neither account declaration nor Export to Nix contains a password or token.
Each new declaration receives fresh random credential references, including after
reusing an account ID. Managed sources remain editable through their Nix declaration.
Export the effective values before importing UI-created accounts into Home Manager:
its declared account list replaces the mutable list on activation.
Removing an account from config disables its connection without silently
deleting mail/drafts/reminders. Provide a separate local-data purge operation.

For app-owned unmanaged config, setup can serialize typed values with
`snix_serde::to_string`, write a temporary sibling `0600`, validate it, and atomically
replace the app-owned file. Do not rewrite arbitrary hand-written Nix expressions
or discard comments/let-bindings: treat an existing user-created file as managed
unless it was explicitly adopted. Preferences do not acquire hidden SQLite
overrides that conflict with Home Manager. Dynamic UI state (selection/window size)
is runtime state, not an override of declarative preferences.

### Export to Nix

Provide a simple Settings action to copy or save a self-contained native Nix
attrset through the clipboard/file portal. Export the supported effective
preferences and account declarations, including `secretRef` identifiers; exclude
passwords, tokens, mailbox contents, runtime cursors, machine UI state and file
ownership metadata. The target deployment supplies `managed`. Present the result
as a snapshot of values, not a reconstruction of the original expression's
comments, functions, imports or let-bindings.

Use `snix_serde::to_string` and evaluate/validate the output with the existing
bounded helper before offering it. Test strings containing quotes, newlines,
backslashes and Nix interpolation syntax, optional/default values and empty lists.
Reimport must preserve all exported values. Exporting never rewrites the active
managed file or activates Home Manager. The user can import the saved attrset as
the module's native `settings`, or install it directly with `xdg.configFile`.
This gives a short declare → edit → export → review/import workflow without a
bidirectional dotfile editor or another configuration language.

## Provider and authentication matrix

| Account | Initial route | Acceptance and caveats |
| --- | --- | --- |
| Google / Gmail / Workspace | Desktop OAuth2 authorization code + S256 PKCE, external browser, loopback listener, `https://mail.google.com/`, SASL XOAUTH2 for IMAP/SMTP | Own registered client; offline access/refresh; revocation and consent denial; Workspace may disable IMAP or app authorization. Public distribution requires the applicable Google verification process. App-password-only is not the promised Google login. |
| iCloud | `imap.mail.me.com:993` implicit TLS; `smtp.mail.me.com:587` required STARTTLS; app-specific password | Apple account with two-factor auth and generated app password; incoming short-name/full-address fallback is user-selectable; outgoing full address. No Apple-account web password collection. |
| Inria | Generic manual IMAP/SMTP, password or configured OAuth2, separate logins/CA/folder map | No reliable public current endpoint/auth source was established in this research. Obtain the user's current IT/Thunderbird settings during P05 live acceptance; do not invent a preset or assume the address is the login. VPN/access policy may apply. |
| PoliTO students | Official student guide currently lists `imap.studenti.polito.it:993` TLS and `smtp.studenti.polito.it:465` TLS, full address login | These are documented student settings, not a guarantee for every affiliation/account. Verify the user's account route before sign-off. |
| PoliTO Exchange Online / staff | Configurable Microsoft delegated OAuth2/PKCE, usually `outlook.office365.com:993` TLS and `smtp.office365.com:587` required STARTTLS where enabled | Institution documents EOL + MFA. Scopes `https://outlook.office.com/IMAP.AccessAsUser.All`, `https://outlook.office.com/SMTP.Send`, `offline_access`; register a public Entra client, tenant/admin consent and SMTP AUTH policy may block it. Do not fall back to Basic or claim Graph/EWS support. |
| UPPA Partage | Official guide: `partage.univ-pau.fr:993` TLS and same host `:465` TLS; login `login@univ-pau.fr` | Address and login may differ. Password route is documented for Partage; confirm current account/MFA policy during live acceptance. |

Sources: [Google XOAUTH2](https://developers.google.com/workspace/gmail/imap/xoauth2-protocol),
[Google desktop flow](https://developers.google.com/identity/protocols/oauth2/native-app),
[Apple settings](https://support.apple.com/en-ie/102525),
[PoliTO student parameters](https://it.polito.it/studenti/il-tuo-account/guida-alla-configurazione-della-posta-elettronica/parametri-per-la-configurazione-della-posta-elettronica),
[PoliTO EOL/MFA](https://it.polito.it/ambiti/it-for-research/altri-strumenti-it-per-la-ricerca),
[Microsoft IMAP/SMTP OAuth](https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth/),
[UPPA Partage FAQ](https://formation.univ-pau.fr/fr/vie-etudiante/outils-numeriques-adresse-mail-etudiante/messagerie-partage.html).
These are configuration evidence, not live login proof.

## Credential lifecycle

Use oo7's standard native/portal-backed encrypted storage. OAuth items also bind
their public client/provider/endpoint/scope metadata; changing it requires a new
authorization rather than reusing a token with another issuer/client. Lookup attributes include
the application ID, stable account ID and `secretRef`; never email contents. Store
refresh tokens/app passwords here; access tokens stay redacted in memory. Serialize
refresh per account and update rotated refresh tokens durably. `invalid_grant`
requires reauthentication; stop retries. A locked/unavailable keyring pauses the
account and offers unlock in the foreground; background mode must not repeatedly
prompt or downgrade to plaintext.

OAuth is an external-browser operation. Bind an ephemeral loopback port before
opening the URL, use random state and S256 PKCE via the oauth2 library, enforce
callback path/state, one-use code and a timeout, then close the listener. Handle
malformed/duplicate/denied callbacks safely. Never render a login form in Blitz or
log the URL's code/token/state. Google desktop registrations can include a
non-confidential client-secret field required by their token endpoint; treat it as
public application metadata, never as a secure secret embedded in a binary. Do not
borrow Thunderbird's/Hylki's OAuth client identifiers. Development/test-user
authorization is not evidence of production consent verification. Flatpak loopback,
browser portal and refresh persistence are separate P05 acceptance checks.
