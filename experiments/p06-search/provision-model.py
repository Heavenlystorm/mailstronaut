"""Explicit development-only download of the committed model artifact manifest."""

import hashlib
import json
import os
import pathlib
import sys
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[2]
assert sys.argv[1:] in [[], ["--potion"], ["--minilm"], ["--nli"]], "Usage: provision-model.py [--potion|--minilm|--nli]"
NLI = sys.argv[1:] == ["--nli"]
POTION = sys.argv[1:] == ["--potion"]
MINILM = sys.argv[1:] == ["--minilm"]
MANIFEST = json.loads((ROOT / "experiments/p06-search" /
                       ("../p09-priority/nli-model-manifest.json" if NLI else
                        "potion-model-manifest.json" if POTION else
                        "minilm-model-manifest.json" if MINILM else "model-manifest.json")).read_text())
DESTINATION = ROOT / "target/p06-search" / ("model-potion" if POTION else
                                          "model-minilm" if MINILM else "model")
if NLI:
    DESTINATION = ROOT / "target/p09-priority/model"
DESTINATION.mkdir(mode=0o700, parents=True, exist_ok=True)
assert DESTINATION.resolve().is_relative_to((ROOT / "target").resolve())
PREFIX = (f"https://huggingface.co/{MANIFEST['modelId']}/resolve/"
          f"{MANIFEST['revision']}/")
EXPECTED = {"model.safetensors", "tokenizer.json", "config.json",
            "tokenizer_config.json", "special_tokens_map.json"}
if NLI:
    EXPECTED |= {"README.md"}
if POTION:
    EXPECTED = {"model.safetensors", "tokenizer.json", "config.json"}
if MINILM:
    EXPECTED |= {"sentence_bert_config.json", "1_Pooling/config.json"}
assert {entry["name"] for entry in MANIFEST["files"]} == EXPECTED


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


for entry in MANIFEST["files"]:
    name, size = entry["name"], entry["sizeBytes"]
    assert entry["url"] == PREFIX + name and 0 < size <= 512 * 1024 * 1024
    output = DESTINATION / name
    if output.exists() or output.is_symlink():
        assert not output.is_symlink() and output.is_file()
        assert output.stat().st_size == size and digest(output) == entry["sha256"]
        print(f"Verified existing {name}: {size} bytes", flush=True)
        continue
    temporary = DESTINATION / (name + ".part")
    temporary.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    # Exclusive creation avoids following an existing path or overwriting evidence.
    with temporary.open("xb") as destination:
        os.chmod(temporary, 0o600)
        checksum = hashlib.sha256()
        received = 0
        request = urllib.request.Request(entry["url"], headers={"User-Agent": "Mailstronaut-development-probe"})
        with urllib.request.urlopen(request, timeout=60) as response:
            assert response.url.startswith("https://"), "HTTPS redirect required"
            while chunk := response.read(min(1024 * 1024, size + 1 - received)):
                received += len(chunk)
                assert received <= size, "Artifact exceeds committed size"
                checksum.update(chunk)
                destination.write(chunk)
        assert received == size and checksum.hexdigest() == entry["sha256"], "Artifact checksum mismatch"
        destination.flush()
        os.fsync(destination.fileno())
    temporary.replace(output)
    print(f"Provisioned and verified {name}: {received} bytes", flush=True)
