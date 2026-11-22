# Flatpak from the current Nix inputs

Use the optional pinned tools without adding Flatpak tools to the default shell:

```sh
nix develop .#flatpak
```

On this project, run these commands on the CI server through `ci/remote.sh`.
The existing isolated installation and SDK provisioning commands are documented
in [the SDK probe](../experiments/p01-flatpak/README.md). Install its four signed
refs at the commits in `experiments/p01-flatpak/sdk-lock.json`, then run its
preflight check. No SDK download happens as part of an application build.

Generate the manifest's two archives directly from the Nix package inputs:

```sh
bash flatpak/prepare.sh
```

This realizes only the standard vendored sources, not a package compilation. The
workspace, package source and vendor lockfiles must agree. The source excludes
the nightly development Cargo settings. The vendor configuration's placeholder
is resolved for the SDK's path, and app/vendor remain siblings. Archives use
normalized metadata and multithreaded XZ compression with fixed block boundaries;
temporary copies are removed on success and failure. Only the two compressed archives are
retained in `target/p10-flatpak/`. The target can reside on the larger CI disk.

When a maintainer changes application source or Cargo.lock, review those changes
and explicitly refresh the archive hashes, then commit the manifest:

```sh
bash flatpak/prepare.sh --update-pins
```

Normal preparation rejects stale pins without changing the manifest or replacing
the previously published archives. The archives exclude `flatpak/`, so updating
their manifest pins does not change their own input. Repeating preparation with
the same pinned inputs must produce the same archive hashes.
XZ's [`--no-adjust` and threaded mode](https://tukaani.org/xz/man/xz.1.html)
allow worker counts to vary without switching to a different stream encoding.

Build and install into the isolated project installation:

```sh
bash flatpak/build-and-install.sh
```

This acquires the shared package-gate lock, prepares/verifies the archives, checks
the installed SDK pins and builds with downloads disabled. It deploys the app
from the local repository using the preinstalled runtime. On a host where the
builder cannot mount `rofiles-fuse`, set
`MAILSTRONAUT_FLATPAK_DISABLE_ROFILES_FUSE=1` to use the builder's copy-based cache.
Its fresh numeric
receipt is `target/p10-flatpak-build.status`; read the complete
`target/p10-flatpak-build.log` as well. `0` proves build/install, not GUI or real
desktop portal acceptance. Use the private native smoke/portal/attachment
walkthroughs separately:

```sh
nix develop .#gui-test --command bash flatpak/check.sh
```

This tests the installed app's CLI/configuration, private Wayland window and
synthetic portal/keyring/attachment flows with network unshared. It uses private
profiles and the existing fixtures, with no real mail. Set
`MAILSTRONAUT_PORTAL_FIXTURE`, `MAILSTRONAUT_SEED_FIXTURE` and
`MAILSTRONAUT_CREDENTIAL_FIXTURE` to reuse compatible
prebuilt workspace fixture binaries. It does not establish the
desktop's own portals or document grants; see [the evidence](../docs/evidence/P10-flatpak.md).

The manifest is native Wayland, with no X11 socket or broad home access. These
local archives are reproducible build inputs, not a published release repository.
Shipping/signing the resulting app and model/portal acceptance remain separate
roadmap tasks.
