"""Offline development-only wording/label-order diagnostic; no held-out inference."""
import hashlib
import json
import os
import pathlib

root = pathlib.Path(__file__).resolve().parents[2]
runtime = root/'target/p09-priority/reference-runtime'
runtime.mkdir(mode=0o700,parents=True,exist_ok=True)
os.environ.update(HF_HUB_OFFLINE='1',HF_HUB_DISABLE_TELEMETRY='1',TRANSFORMERS_OFFLINE='1',
                  CUDA_VISIBLE_DEVICES='',HF_HOME=str(runtime/'hf'),TMPDIR=str(runtime),TORCH_HOME=str(runtime/'torch'))
import torch
from transformers import AutoModelForSequenceClassification, AutoTokenizer

directory = root/'target/p09-priority/model'
manifest = json.loads((root/'experiments/p09-priority/nli-model-manifest.json').read_text())
for artifact in manifest['files']:
    with (directory/artifact['name']).open('rb') as source:
        assert hashlib.file_digest(source,'sha256').hexdigest() == artifact['sha256']
torch.set_num_threads(os.cpu_count())
model = AutoModelForSequenceClassification.from_pretrained(directory,local_files_only=True,
    trust_remote_code=False,use_safetensors=True,attn_implementation='eager').to('cpu').eval()
tokenizer = AutoTokenizer.from_pretrained(directory,local_files_only=True,trust_remote_code=False)
config = json.loads((root/'experiments/p09-priority/hypothesis-candidates.json').read_text())

def scores(premise,hypothesis):
    with torch.inference_mode():
        encoded = tokenizer(premise,hypothesis,max_length=512,truncation='only_first',return_tensors='pt')
        logits = model(**encoded).logits[0]
        return dict(three_way=torch.softmax(logits,dim=-1).tolist(),
                    entailment_vs_contradiction=torch.softmax(logits[[0,2]],dim=-1)[0].item())

result = dict(controls=[],development=[])
for control in config['controls']:
    record = dict(expected=control['expected'],scores=scores(control['premise'],control['hypothesis']))
    result['controls'].append(record)
    print('control',json.dumps(record),flush=True)
source = json.loads((root/'experiments/p09-priority/priority-corpus.json').read_text())
selected = [row for row in source['messages'] if row['split']=='development'
            and row['group'] in config['groups'] and row['id'].endswith('-0')]
assert len(selected)==9
for variant in config['variants']:
    for row in selected:
        # These nine bare examples have no quotes/signatures. No duplicate quote parser.
        premise = row['subject']+'\n\n'+row['body']
        observations = [scores(premise,hypothesis) for hypothesis in variant['hypotheses']]
        record = dict(variant=variant['name'],id=row['id'],scores=observations)
        result['development'].append(record)
        print(json.dumps(record),flush=True)
(root/'target/p09-priority/hypothesis-diagnostic.json').write_text(json.dumps(result,indent=2)+'\n')
print('Diagnostic complete; no policy adoption or held-out result.',flush=True)
