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

import numpy as np
import torch
import transformers
from transformers import AutoModel, AutoTokenizer

model_path = ROOT / "target/p06-search/model"
observed_path = pathlib.Path(sys.argv[1]).resolve()
assert observed_path.is_relative_to((ROOT / "target/p06-search").resolve())
observed = json.loads(observed_path.read_text())
manifest = json.loads((ROOT / "experiments/p06-search/model-manifest.json").read_text())
for artifact in manifest["files"]:
    path = model_path / artifact["name"]
    assert path.stat().st_size == artifact["sizeBytes"]
    with path.open("rb") as source:
        assert hashlib.file_digest(source, "sha256").hexdigest() == artifact["sha256"]

torch.set_num_threads(os.cpu_count())
tokenizer = AutoTokenizer.from_pretrained(model_path, local_files_only=True, trust_remote_code=False)
model = AutoModel.from_pretrained(model_path, local_files_only=True, trust_remote_code=False,
                                  use_safetensors=True, attn_implementation="eager").to("cpu").eval()
fixtures = json.loads((ROOT / "experiments/p06-search/relevance.json").read_text())
print(f"torch={torch.__version__} transformers={transformers.__version__} device=cpu", flush=True)

for name, prefix in [("documents", "passage: "), ("queries", "query: ")]:
    references = []
    with torch.inference_mode():
        for item in fixtures[name]:
            inputs = tokenizer(prefix + item["text"], max_length=512, truncation=True,
                               return_tensors="pt")
            hidden = model(**inputs).last_hidden_state
            mask = inputs["attention_mask"].unsqueeze(-1).bool()
            pooled = hidden.masked_fill(~mask, 0.0).sum(1) / mask.sum(1)
            vector = torch.nn.functional.normalize(pooled, p=2, dim=1)[0].numpy()
            references.append(vector)
    reference = np.stack(references)
    actual = np.array(observed[name], dtype=np.float32)
    assert reference.shape == actual.shape and np.isfinite(actual).all()
    error = np.max(np.abs(reference - actual))
    cosine = np.min(np.sum(reference * actual, axis=1))
    print(f"{name}: max_abs_error={error:.8g} min_cosine={cosine:.8g}", flush=True)
    # Fixed before observing output. F32 CPU implementations may sum differently.
    assert error <= 1e-4 and cosine >= 0.99999, "Independent embedding parity failed"
