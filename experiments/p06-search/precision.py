"""Frozen cutoff screening; independent local oracle for new no-answer vectors."""
import hashlib
import json
import os
import pathlib
import sys

root = pathlib.Path(__file__).resolve().parents[2]
os.environ.update(HF_HUB_OFFLINE="1", HF_HUB_DISABLE_TELEMETRY="1", TRANSFORMERS_OFFLINE="1", CUDA_VISIBLE_DEVICES="", HF_HOME=str(root/"target/p06-search/reference-cache"))
runtime = root/"target/p06-search/reference-runtime"
runtime.mkdir(mode=0o700, parents=True, exist_ok=True)
os.environ.update(TMPDIR=str(runtime), TORCH_HOME=str(runtime/"torch"))
import numpy as np
import torch
from transformers import AutoModel, AutoTokenizer

output = pathlib.Path(sys.argv[1])
cutoff = float(sys.argv[2]) if len(sys.argv)>2 else .5
assert 0 <= cutoff <= 1
print(f"Frozen cutoff: {cutoff}",flush=True)
model_path = root/"target/p06-search/model-minilm"
manifest = json.loads((root/"experiments/p06-search/minilm-model-manifest.json").read_text())
for artifact in manifest["files"]:
    with (model_path/artifact["name"]).open("rb") as source:
        assert hashlib.file_digest(source,"sha256").hexdigest()==artifact["sha256"]
torch.set_num_threads(os.cpu_count())
model = AutoModel.from_pretrained(model_path, local_files_only=True, trust_remote_code=False, use_safetensors=True, attn_implementation="eager").to("cpu").eval()
tokenizer = AutoTokenizer.from_pretrained(model_path,local_files_only=True,trust_remote_code=False)
negative = json.loads((root/"experiments/p06-search/no-answer.json").read_text())["queries"]
actual = np.array(json.loads((output/"negative.json").read_text())["queries"],dtype=np.uint32).view(np.float32)
reference = []
with torch.inference_mode():
    for query in negative:
        encoded = tokenizer(query["text"],max_length=128,truncation=True,return_tensors="pt")
        hidden = model(**encoded).last_hidden_state
        mask = encoded["attention_mask"].unsqueeze(-1).bool()
        pooled = hidden.masked_fill(~mask,0.).sum(1)/mask.sum(1)
        reference.append(torch.nn.functional.normalize(pooled,p=2,dim=1)[0].numpy())
reference = np.stack(reference)
assert np.max(np.abs(actual-reference)) <=1e-4
assert np.min(np.sum(actual*reference,axis=1)) >=.99999
print("New no-answer Rust/reference parity passed",flush=True)
failures = set()
for name,fixture_name,minimum in [("fixed","relevance.json",(8,10)),("heldout","relevance-heldout.json",(20,23))]:
    fixture = json.loads((root/"experiments/p06-search"/fixture_name).read_text())
    vectors = json.loads((output/f"{name}.json").read_text())
    eligible = [i for i,item in enumerate(fixture["documents"]) if item["account"]=="a" and item["folder"]=="INBOX"]
    documents = np.array(vectors["documents"],dtype=np.uint32).view(np.float32)[eligible]
    queries = np.array(vectors["queries"],dtype=np.uint32).view(np.float32)
    first = three = 0
    for query,scores in zip(fixture["queries"],queries@documents.T,strict=True):
        hits = [fixture["documents"][eligible[i]]["id"] for i in np.argsort(-scores,kind="stable") if scores[i]>=cutoff]
        rank = hits.index(query["expected"])+1 if query["expected"] in hits else None
        first += rank==1
        three += rank is not None and rank<=3
        if query["id"]=="user-example": assert rank==1
        print(f"{name} {query['id']}: expected-rank={rank}",flush=True)
    print(f"{name}: first={first} top3={three} minimum={minimum}",flush=True)
    assert first>=minimum[0] and three>=minimum[1]
    for query,scores in zip(negative,actual@documents.T,strict=True):
        maximum = float(scores.max())
        if maximum>=cutoff: failures.add(query["id"])
        print(f"{name} no-answer {query['id']}: maximum-cosine={maximum:.6f}",flush=True)
print(f"No-answer queries with any hit at frozen cutoff: {sorted(failures)}",flush=True)
assert len(failures)<=1
