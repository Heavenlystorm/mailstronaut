"""Independent, test-only local CPU oracle. No Python runtime is added to the app."""

import hashlib
import json
import os
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
os.environ["HF_HUB_OFFLINE"] = "1"
os.environ["HF_HUB_DISABLE_TELEMETRY"] = "1"
os.environ["TRANSFORMERS_OFFLINE"] = "1"
os.environ["HF_HOME"] = str(ROOT / "target/p06-search/reference-cache")
runtime = ROOT / "target/p06-search/reference-runtime"
runtime.mkdir(mode=0o700, parents=True, exist_ok=True)
os.environ["TMPDIR"] = str(runtime)
os.environ["TORCH_HOME"] = str(runtime / "torch")
os.environ["CUDA_VISIBLE_DEVICES"] = ""

import numpy as np
import torch
import transformers
from transformers import AutoModel, AutoTokenizer

assert sys.argv[2:] in [[], ["--minilm"], ["--minilm", "--held-out"]]
minilm = "--minilm" in sys.argv[2:]
held_out = "--held-out" in sys.argv[2:]
model_path = ROOT / "target/p06-search" / ("model-minilm" if minilm else "model")
observed_path = pathlib.Path(sys.argv[1]).resolve()
assert observed_path.is_relative_to((ROOT / "target/p06-search").resolve())
observed = json.loads(observed_path.read_text())
manifest = json.loads((ROOT / "experiments/p06-search" /
                       ("minilm-model-manifest.json" if minilm else "model-manifest.json")).read_text())
for artifact in manifest["files"]:
    path = model_path / artifact["name"]
    assert path.stat().st_size == artifact["sizeBytes"]
    with path.open("rb") as source:
        assert hashlib.file_digest(source, "sha256").hexdigest() == artifact["sha256"]

torch.set_num_threads(os.cpu_count())
tokenizer = AutoTokenizer.from_pretrained(model_path, local_files_only=True, trust_remote_code=False)
model = AutoModel.from_pretrained(model_path, local_files_only=True, trust_remote_code=False,
                                  use_safetensors=True, attn_implementation="eager").to("cpu").eval()
fixture_name = "relevance-heldout.json" if held_out else "relevance.json"
fixtures = json.loads((ROOT / "experiments/p06-search" / fixture_name).read_text())
print(f"torch={torch.__version__} transformers={transformers.__version__} device=cpu", flush=True)

actual_outputs = {}
for name, prefix in [("documents", manifest["prefixes"]["passage"]),
                     ("queries", manifest["prefixes"]["query"])]:
    references = []
    with torch.inference_mode():
        for item in fixtures[name]:
            inputs = tokenizer(prefix + item["text"], max_length=manifest["maxTokens"], truncation=True,
                               return_tensors="pt")
            hidden = model(**inputs).last_hidden_state
            mask = inputs["attention_mask"].unsqueeze(-1).bool()
            pooled = hidden.masked_fill(~mask, 0.0).sum(1) / mask.sum(1)
            vector = torch.nn.functional.normalize(pooled, p=2, dim=1)[0].numpy()
            references.append(vector)
    reference = np.stack(references)
    actual = np.array(observed[name], dtype=np.uint32).view(np.float32)
    actual_outputs[name] = actual
    assert reference.shape == actual.shape and np.isfinite(actual).all()
    error = np.max(np.abs(reference - actual))
    cosine = np.min(np.sum(reference * actual, axis=1))
    print(f"{name}: max_abs_error={error:.8g} min_cosine={cosine:.8g}", flush=True)
    # Fixed before observing output. F32 CPU implementations may sum differently.
    assert error <= 1e-4 and cosine >= 0.99999, "Independent embedding parity failed"

if minilm:
    # The original screen, now computed from independently checked Rust vectors.
    eligible = [index for index, item in enumerate(fixtures["documents"])
                if item["account"] == "a" and item["folder"] == "INBOX"]
    scores = actual_outputs["queries"] @ actual_outputs["documents"][eligible].T
    top1 = top3 = 0
    example_rank = None
    for query, similarities in zip(fixtures["queries"], scores, strict=True):
        ids = [fixtures["documents"][eligible[index]]["id"]
               for index in np.argsort(-similarities, kind="stable")]
        rank = ids.index(query["expected"]) + 1
        top1 += rank == 1
        top3 += rank <= 3
        if query["id"] == "user-example":
            example_rank = rank
        print(f"Rust vectors: {query['id']} rank={rank} top3={ids[:3]}", flush=True)
    print(f"Rust-vector screen: top1={top1}/{len(fixtures['queries'])} "
          f"top3={top3}/{len(fixtures['queries'])} example_rank={example_rank}", flush=True)
    passed = ((top1 >= 20 and top3 >= 23 and len(fixtures['queries']) == 24) if held_out
              else (top1 >= 8 and top3 == 10 and example_rank == 1))
    assert passed, "Rust-vector quality screen failed"
