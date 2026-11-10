"""Quality-first CPU oracle: reject poor encoders before a new Rust dependency build."""

import argparse
import hashlib
import json
import os
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser()
parser.add_argument("model", choices=["e5", "minilm"])
parser.add_argument("--held-out", action="store_true")
args = parser.parse_args()
name = "minilm-model-manifest.json" if args.model == "minilm" else "model-manifest.json"
manifest = json.loads((ROOT / "experiments/p06-search" / name).read_text())
model_path = (ROOT / "target/p06-search" /
              ("model-minilm" if args.model == "minilm" else "model")).resolve()
assert model_path.is_relative_to((ROOT / "target").resolve())
for artifact in manifest["files"]:
    path = model_path / artifact["name"]
    assert not path.is_symlink() and path.is_file() and path.resolve().is_relative_to(model_path)
    assert path.stat().st_size == artifact["sizeBytes"]
    with path.open("rb") as source:
        assert hashlib.file_digest(source, "sha256").hexdigest() == artifact["sha256"]
if args.model == "minilm":
    settings = json.loads((model_path / "sentence_bert_config.json").read_text())
    pooling = json.loads((model_path / "1_Pooling/config.json").read_text())
    assert settings["max_seq_length"] == manifest["maxTokens"] == 128
    assert pooling["word_embedding_dimension"] == manifest["dimensions"] == 384
    assert pooling["pooling_mode_mean_tokens"]
    assert all(not value for key, value in pooling.items()
               if key.startswith("pooling_mode_") and key != "pooling_mode_mean_tokens")

runtime = ROOT / "target/p06-search/reference-runtime"
runtime.mkdir(mode=0o700, parents=True, exist_ok=True)
os.environ.update(HF_HUB_OFFLINE="1", TRANSFORMERS_OFFLINE="1",
                  HF_HUB_DISABLE_TELEMETRY="1", CUDA_VISIBLE_DEVICES="",
                  TMPDIR=str(runtime), TORCH_HOME=str(runtime / "torch"),
                  HF_HOME=str(ROOT / "target/p06-search/reference-cache"))
import numpy as np
import torch
import transformers
from transformers import AutoModel, AutoTokenizer

torch.set_num_threads(os.process_cpu_count() or 1)
model = AutoModel.from_pretrained(model_path, local_files_only=True, trust_remote_code=False,
                                  use_safetensors=True, attn_implementation="eager").to("cpu").eval()
tokenizer = AutoTokenizer.from_pretrained(model_path, local_files_only=True, trust_remote_code=False)
fixture_name = "relevance-heldout.json" if args.held_out else "relevance.json"
fixtures = json.loads((ROOT / "experiments/p06-search" / fixture_name).read_text())
documents = [item for item in fixtures["documents"]
             if item["account"] == "a" and item["folder"] == "INBOX"]


def encode(items, prefix):
    vectors = []
    with torch.inference_mode():
        for item in items:
            inputs = tokenizer(prefix + item["text"], truncation=True,
                               max_length=manifest["maxTokens"], return_tensors="pt")
            hidden = model(**inputs).last_hidden_state
            mask = inputs["attention_mask"].unsqueeze(-1).bool()
            pooled = hidden.masked_fill(~mask, 0.0).sum(1) / mask.sum(1)
            vectors.append(torch.nn.functional.normalize(pooled, p=2, dim=1)[0].numpy())
    matrix = np.stack(vectors)
    assert matrix.shape == (len(items), manifest["dimensions"]) and np.isfinite(matrix).all()
    return matrix


print(f"model={manifest['modelId']} revision={manifest['revision']}", flush=True)
print(f"torch={torch.__version__} transformers={transformers.__version__} device=cpu", flush=True)
scores = (encode(fixtures["queries"], manifest["prefixes"]["query"]) @
          encode(documents, manifest["prefixes"]["passage"]).T)
top_one = top_three = 0
example_first = False
for query, similarities in zip(fixtures["queries"], scores, strict=True):
    ids = [documents[index]["id"] for index in np.argsort(-similarities, kind="stable")]
    rank = ids.index(query["expected"]) + 1
    top_one += rank == 1
    top_three += rank <= 3
    if query["id"] == "user-example":
        example_first = rank == 1
    print(json.dumps(dict(query=query["id"], expected=query["expected"], rank=rank, top3=ids[:3])), flush=True)
passed = ((top_one >= 20 and top_three >= 23 and len(fixtures["queries"]) == 24)
          if args.held_out else
          (top_one >= 8 and top_three == len(fixtures["queries"]) and example_first))
print(json.dumps(dict(top1=top_one, top3=top_three, total=len(fixtures["queries"]),
                      exampleFirst=None if args.held_out else example_first, passed=passed)), flush=True)
print(f"fixed-screen-exit={0 if passed else 1}", flush=True)
raise SystemExit(0 if passed else 1)
