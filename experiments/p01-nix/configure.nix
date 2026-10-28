# Configure-only native-tool probe. No Rust workspace or SpiderMonkey compilation.
let
  flake = builtins.getFlake "git+file://${toString ../..}";
  package = flake.packages.x86_64-linux.mailstronaut;
in
package.overrideAttrs (_old: {
  name = "mailstronaut-mozjs-configure-probe";
  cargoDeps = package.cargoDeps;
  buildPhase = ''
    runHook preBuild
    crate="$NIX_BUILD_TOP/$(stripHash "$cargoDeps")/source-registry-0/mozjs_sys-153.3.0-1"
    mkdir "$NIX_BUILD_TOP/mozjs-configure"
    cd "$NIX_BUILD_TOP/mozjs-configure"
    export MOZBUILD_STATE_PATH="$PWD/mozbuild-state"
    export PYTHONDONTWRITEBYTECODE=1
    export NO_RUST_PANIC_HOOK=1
    export CARGO_FEATURE_INTL=1
    unset CARGO_FEATURE_JIT
    export HOST=x86_64-unknown-linux-gnu
    export TARGET=x86_64-unknown-linux-gnu
    export PROFILE=release
    export SRC_DIR="$crate/mozjs"
    make -R -f "$crate/makefile.cargo" maybe-configure
  '';
  doCheck = false;
  installPhase = ''
    mkdir -p "$out"
    cp "$NIX_BUILD_TOP/mozjs-configure/config.log" "$out/"
    cp "$NIX_BUILD_TOP/mozjs-configure/config.status" "$out/"
  '';
  postInstall = "";
  postFixup = "";
})
