# Explicit model installation, independent of the application. Evaluation and
# startup never fetch weights; only an explicit build/installation does so.
{ pkgs }:
let
  manifestPath = ../crates/mailstronaut-models/models/minilm.json;
  manifest = builtins.fromJSON (builtins.readFile manifestPath);
  artifacts = map (artifact: {
    inherit (artifact) name;
    path = pkgs.fetchurl {
      inherit (artifact) url hash;
    };
  }) manifest.files;
in
(pkgs.linkFarm "mailstronaut-minilm-${manifest.revision}" (
  artifacts
  ++ [
    {
      name = "manifest.json";
      path = manifestPath;
    }
    {
      name = "LICENSE";
      path = ../crates/mailstronaut-models/models/LICENSE-MiniLM;
    }
  ]
)).overrideAttrs
  (_old: {
    meta = {
      description = "Pinned local multilingual MiniLM sentence encoder for Mailstronaut";
      homepage = "https://huggingface.co/${manifest.modelId}";
      license = pkgs.lib.licenses.asl20;
    };
    passthru = {
      inherit (manifest) revision;
      modelId = manifest.modelId;
    };
  })
