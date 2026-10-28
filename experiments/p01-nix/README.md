# P01 native package probes

The normal acceptance gate is `nix flake check`. For a long local build, run it
through the standard transient user service below so closing or restarting the
coding UI does not disconnect Nix. The host must remain running. The service has
no installed unit file or Home Manager activation.

From the project root, first inspect any existing run:

```sh
systemctl --user show mailstronaut-p01-nix-gate.service \
  -p ActiveState -p SubState -p MainPID
```

If it is active, wait for that run. Otherwise start one:

```sh
systemd-run --user --unit=mailstronaut-p01-nix-gate --collect --service-type=exec \
  --property="WorkingDirectory=$PWD" \
  --setenv=PATH=/run/current-system/sw/bin:/usr/bin:/bin \
  /run/current-system/sw/bin/bash "$PWD/experiments/p01-nix/run-gate.sh"
```

The runner takes an exclusive project-local lock and runs the unchanged pinned
gate with `--cores 2 --max-jobs 1`. It writes:

- `target/p01-package-gate-service.log`: complete Nix/build/check output.
- `target/p01-package-gate.status`: `running`, then the numeric exit status.

Wait for an inactive service **and** a numeric status. Zero establishes the local
gate; a nonzero value is a failure to inspect in the log. `running` with an
inactive/missing service is an interrupted run, including after a host reboot.
Preserve the previous log before restarting an interrupted service. The transient
unit is collected after it exits. This is a local build helper, not CI-container
or installed-application acceptance.

After the gate passes, build the cached output and run the installed CLI probe:

```sh
nix build --no-update-lock-file
nix develop --command bash experiments/p01-nix/installed-smoke.sh \
  "$(readlink -f result)"
```

The installed probe invokes the package with an empty environment and an unusable
executable search path. It checks version, native configuration evaluation,
private initialization, rejection of overwriting a file, and schema rejection.
Artifacts remain in a fresh `target/p01-installed-smoke.*` directory. This does
not start the GUI or establish graphical closure, portals or live account use.

`nested-metadata.nix` and `configure.nix` retain the earlier bounded native-source
experiments described in [P01 evidence](../../docs/evidence/P01.md). Their success
is separate from the completed application gate and installed smoke probe.
