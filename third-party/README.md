# Exact upstream source cache

`snix-f393b22.git.tar.xz` is an unchanged, single-commit shallow Git repository of
Snix, cached because its forge repeatedly returned HTTP 504 to clean CI runners.
It contains the complete upstream tree and original commit object, without local
credentials, remotes or history. It is a source cache, not an application fork.

- Upstream: https://git.snix.dev/snix/snix.git
- Commit: `f393b22aa9b8e0e74c9e8d67f15454409299e510`
- Tree: `45d3546fc8de18b759530d4d324812fc6e3272de`
- Archive SHA-256: `6e8aeec40a2db3246484e8dc30d9e59116efce75a90cb7444692bfd6bcf00e68`
- License: upstream GPL-3.0 and the exceptions recorded in its source tree. The
  full upstream license files are present in the cached tree (`git show HEAD:snix/LICENSE`).

The Nix vendor fetch uses Git's standard URL rewrite for this one upstream URL,
checks the original commit and tree, and leaves Cargo.lock's source identity
unchanged. Nix's recursive vendor hash still verifies the resulting sources.
Other Git and registry sources use the normal Nixpkgs fetcher. The cache is not
shipped in the installed application and does not contain compiled artifacts.

To refresh after deliberately changing the pin, clone the exact upstream commit
into a new shallow bare repository, remove its origin, verify `git fsck --full`,
and archive the repository with deterministic tar metadata. Check the commit and
tree against the trusted upstream checkout before updating the Nix checks/hash.
Do not substitute a different mirror revision merely to make a build pass.
