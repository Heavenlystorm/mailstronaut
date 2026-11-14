"""Offline, test-only full-logit oracle for the frozen Candle NLI API probe."""
import hashlib
import json
import os
import pathlib

root = pathlib.Path(__file__).resolve().parents[2]
runtime = root / 'target/p09-priority/reference-runtime'
runtime.mkdir(mode=0o700, parents=True, exist_ok=True)
os.environ.update(HF_HUB_OFFLINE='1', HF_HUB_DISABLE_TELEMETRY='1',
                  TRANSFORMERS_OFFLINE='1', CUDA_VISIBLE_DEVICES='',
                  HF_HOME=str(runtime/'hf'), TMPDIR=str(runtime), TORCH_HOME=str(runtime/'torch'))

import numpy as np
import torch
from transformers import AutoModelForSequenceClassification, AutoTokenizer

model_path = root/'target/p09-priority/model'
manifest = json.loads((root/'experiments/p09-priority/nli-model-manifest.json').read_text())
for artifact in manifest['files']:
    path = model_path/artifact['name']
    assert path.is_file() and not path.is_symlink() and path.stat().st_size == artifact['sizeBytes']
    with path.open('rb') as source:
        assert hashlib.file_digest(source, 'sha256').hexdigest() == artifact['sha256']
torch.set_num_threads(os.cpu_count())
model = AutoModelForSequenceClassification.from_pretrained(
    model_path, local_files_only=True, trust_remote_code=False,
    use_safetensors=True, attn_implementation='eager').to('cpu').eval()
assert model.config.id2label == {0:'entailment', 1:'neutral', 2:'contradiction'}
tokenizer = AutoTokenizer.from_pretrained(model_path, local_files_only=True, trust_remote_code=False)
pairs = json.loads((root/'experiments/p09-priority/nli-pairs.json').read_text())
observed = json.loads((root/'target/p09-priority/parity/observed.json').read_text())
assert len(pairs) == len(observed)
outputs = {}
with torch.inference_mode():
    for pair, actual in zip(pairs, observed, strict=True):
        assert pair['id'] == actual['id']
        encoded = tokenizer(pair['premise'], pair['hypothesis'], max_length=512,
                            truncation='only_first', return_token_type_ids=True)
        if pair.get('pad_to'):
            encoded = tokenizer.pad(encoded, padding='max_length', max_length=pair['pad_to'])
        assert encoded['input_ids'] == actual['input_ids']
        assert encoded['attention_mask'] == actual['attention_mask']
        assert set(encoded['token_type_ids']) == {0}
        inputs = {key:torch.tensor([value], dtype=torch.long) for key,value in encoded.items()}
        reference = model(**inputs).logits[0].numpy()
        rust = np.array(actual['logits_bits'], dtype=np.uint32).view(np.float32)
        assert rust.shape == reference.shape == (3,) and np.isfinite(rust).all()
        error = float(np.max(np.abs(reference-rust)))
        print(f"{pair['id']}: tokens={len(actual['input_ids'])} max_abs_logit_error={error:.9g}", flush=True)
        assert error <= 1e-4, 'Frozen full-logit tolerance failed'
        outputs[pair['id']] = rust
for unpadded,padded in [('fr-action','fr-padding'), ('it-action','it-padding')]:
    error = float(np.max(np.abs(outputs[unpadded]-outputs[padded])))
    print(f'{unpadded} padding invariant: {error:.9g}', flush=True)
    assert error <= 1e-4
assert observed[-1]['truncated'] and len(observed[-1]['input_ids']) == 512
print('Full-logit, pair-tokenization, padding and bounded-overflow parity passed; not priority quality acceptance.', flush=True)
