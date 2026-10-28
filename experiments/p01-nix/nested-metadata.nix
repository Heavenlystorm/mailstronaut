# Focused offline cbindgen metadata probe; run with nix build --impure --file this-file.
let
  flake = builtins.getFlake "git+file://${toString ../..}";
  package = flake.packages.x86_64-linux.mailstronaut;
in
package.overrideAttrs (_old: {
  name = "mailstronaut-nested-metadata-probe";
  cargoDeps = package.cargoDeps;
  buildPhase = ''
    runHook preBuild
    mkdir -p "$NIX_BUILD_TOP/metadata-results"
    for crate in "$NIX_BUILD_TOP/$(stripHash "$cargoDeps")"/source-registry-0/mozjs_{locale,properties,collator,normalizer}_glue-* \
      "$NIX_BUILD_TOP/$(stripHash "$cargoDeps")"/source-registry-0/mozjs_unicode_bidi_ffi-*; do
      (
        cd "$crate"
        cargo metadata --offline --all-features --format-version 1 --manifest-path ./Cargo.toml \
          > "$NIX_BUILD_TOP/metadata-results/$(basename "$crate").json"
      )
    done
  '';
  doCheck = false;
  installPhase = ''
    mkdir -p "$out"
    cp "$NIX_BUILD_TOP/metadata-results/"*.json "$out/"
  '';
  postInstall = "";
  postFixup = "";
})
