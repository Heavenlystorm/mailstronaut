"""Evaluate frozen synthetic observations; never loads a model or encodes held-out mail."""
import hashlib
import json
import os
import pathlib
import sys

os.environ.update(HF_HUB_OFFLINE='1', TRANSFORMERS_OFFLINE='1', CUDA_VISIBLE_DEVICES='')
import numpy as np
import torch

root = pathlib.Path(__file__).resolve().parents[2]
assert sys.argv[1:] in [['development'], ['heldout']]
split = sys.argv[1]
directory = root/'target/p09-priority'/split
report = json.loads((directory/'observed.json').read_text())
policy_path = root/'experiments/p09-priority/nli-policy.json'
policy = json.loads(policy_path.read_text())
assert report['split'] == split and report['rules_policy'] == policy['rulesPolicy']
assert report['hypotheses'] == policy['hypotheses'] and report['projection_policy'] == policy['projectionPolicy']
assert report['model_manifest_sha256'] == hashlib.sha256((root/'experiments/p09-priority/nli-model-manifest.json').read_bytes()).hexdigest()
assert report['rules_source_sha256'] == policy['rulesSourceSha256']
threshold = {key: float(np.float32(value)) for key,value in policy.items()
             if isinstance(value,float)}
gold = {}
for name,digest in [('priority-corpus.json','a2ff10ab4315b03c8a185301baff00f90f8f15ae4038c308ead8b59e268306ca'),
                    ('semantic-challenges.json','8cf9d92868dd23c17574d8caddbda0f064151e33c9539f815d73d528222eaab4')]:
    raw = (root/'experiments/p09-priority'/name).read_bytes()
    assert hashlib.sha256(raw).hexdigest() == digest
    for message in json.loads(raw)['messages']:
        if message['split'] == split:
            gold[message['id']] = message
assert set(gold) == {row['id'] for row in report['rows']}
outcomes = []
for row in report['rows']:
    source = gold[row['id']]
    assert row['expected_priority'] == source['expected_priority']
    assert row['expected_deadline'] == source['expected_deadline']
    assert row['lang'] == source['lang'] and row['group'] == source['group']
    assert len(row['observations']) == 4
    assert [item['id'] for item in row['observations']] == [f"{row['id']}:{index}" for index in range(4)]
    logits = np.array([item['logits_bits'] for item in row['observations']],dtype=np.uint32).view(np.float32)
    assert logits.shape == (4,3) and np.isfinite(logits).all()
    probabilities = torch.softmax(torch.tensor(logits,device='cpu'),dim=-1).tolist()
    action,neutral,contradiction = probabilities[0]
    urgency,information = probabilities[1][0],probabilities[2][0]
    rules = row['rules']
    guarded = rules['status'] != 'RulesOnly' or any(reason in rules['reasons'] for reason in ('NoActionNeeded','AlreadyExpired'))
    future = [item for item in rules['deadlines'] if item['ends_at'] is not None and item['ends_at']>row['as_of']]
    deadline = min(future,key=lambda item:item['ends_at'])['date'] if future else None
    baseline = rules['priority'] and rules['expires_at'] is not None and rules['expires_at']>row['as_of']
    positive = action>=threshold['actionMin'] and action-neutral>=threshold['actionMargin'] and (urgency>=threshold['urgencyMin'] or bool(future))
    negative = ((contradiction>=threshold['contradictionMin'] and contradiction-neutral>=threshold['contradictionMargin'])
                or (information>=threshold['informationMin'] and action<=threshold['vetoActionMax']))
    conflicting = information>=threshold['informationMin'] and action>threshold['vetoActionMax']
    if guarded:
        decision,result,predicted_deadline = 'guarded',False,None
    elif conflicting:
        decision,result,predicted_deadline = 'rules_only',baseline,deadline
    elif positive:
        decision,result,predicted_deadline = 'model_positive',True,deadline
    elif negative:
        decision,result,predicted_deadline = 'model_negative',False,None
    else:
        decision,result,predicted_deadline = 'rules_only',baseline,deadline
    outcomes.append(dict(id=row['id'],lang=row['lang'],group=row['group'],expected=row['expected_priority'],
                         expected_deadline=row['expected_deadline'],baseline=baseline,model_only=positive and not guarded and not conflicting,
                         combined=result,decision=decision,rule_deadline=deadline,predicted_deadline=predicted_deadline,
                         probabilities=probabilities,micros=row['micros'],truncated=any(item['truncated'] for item in row['observations'])))

def metrics(rows,field):
    tp=sum(row[field] and row['expected'] for row in rows)
    fp=sum(row[field] and not row['expected'] for row in rows)
    fn=sum(not row[field] and row['expected'] for row in rows)
    precision=tp/(tp+fp) if tp+fp else None
    recall=tp/(tp+fn) if tp+fn else None
    f1=2*tp/(2*tp+fp+fn) if 2*tp+fp+fn else None
    return dict(tp=tp,fp=fp,fn=fn,precision=precision,recall=recall,f1=f1)

summaries=[]
for subset in ('all','original','challenges'):
    for language in ('en','fr','it'):
        rows=[row for row in outcomes if row['lang']==language and
              (subset=='all' or row['group'].startswith('challenge-')==(subset=='challenges'))]
        if not rows: continue
        entry=dict(subset=subset,lang=language,count=len(rows),
                   baseline=metrics(rows,'baseline'),combined=metrics(rows,'combined'),model_only=metrics(rows,'model_only'),
                   abstentions=sum(row['decision']=='rules_only' for row in rows),
                   corrections=sum(row['combined']==row['expected'] and row['baseline']!=row['expected'] for row in rows),
                   regressions=sum(row['combined']!=row['expected'] and row['baseline']==row['expected'] for row in rows))
        found=[row for row in rows if row['predicted_deadline'] is not None]
        baseline_dates=[row for row in rows if row['rule_deadline'] is not None]
        entry['baseline_deadline_accepted']=len(baseline_dates)
        entry['baseline_deadline_correct']=sum(row['rule_deadline']==row['expected_deadline'] for row in baseline_dates)
        entry['deadline_accepted']=len(found)
        entry['deadline_correct']=sum(row['predicted_deadline']==row['expected_deadline'] for row in found)
        entry['deadline_precision']=entry['deadline_correct']/len(found) if found else None
        summaries.append(entry)
        print(json.dumps(entry),flush=True)
timings=sorted(row['micros']/1000 for row in outcomes)
print(f"{split}: messages={len(outcomes)} p95_ms={np.percentile(timings,95):.3f} excerpt_only={sum(row['truncated'] for row in outcomes)}",flush=True)
result=dict(split=split,policy=policy,policy_sha256=hashlib.sha256(policy_path.read_bytes()).hexdigest(),summaries=summaries,outcomes=outcomes)
(directory/'evaluation.json').write_text(json.dumps(result,indent=2)+'\n')
print('Evaluation completed. This is synthetic policy research, not application adoption.',flush=True)
