# Frozen relevance-cutoff screen

2026-11-13. The first native GUI walkthrough retrieved the correct French notice
first, but displayed unrelated tail results. No similarity scores have yet been
examined to select a cutoff. Before evaluating this additional screen, fix the
candidate at **cosine similarity >=0.5** (USearch cosine distance <=0.5).

Adopt it only if the unchanged original relevance gates still pass (>=8/10 first,
10/10 top-three, the doctoral example first), the unchanged held-out gates still
pass (>=20/24 first and >=23/24 top-three), and at most one of the twelve frozen
no-answer queries returns any result against either original or held-out scoped
corpus. All twelve concern absent cinema, guitar, recipe or chess correspondence.
Also require the native four-message doctoral walkthrough to retain the French
notice while excluding its library/department/budget distractors.

Keep failed outcomes and these datasets unchanged. These are synthetic screening
criteria, not proof of perfect no-answer handling or a universal cosine cutoff.
This proposes a filter over the existing library metric, not a new ranker/model.

## First candidate and separate calibration rule

The fixed 0.5 candidate failed the unchanged original screen: 8/10 first and
8/10 top-three, dropping two required cross-language positives. It is rejected.
The run stopped there; neither the held-out cutoff outcomes nor no-answer
similarities were examined.

Before the next evaluation, fix a development-only calibration rule: take the
lowest expected-positive cosine among the original ten development queries,
subtract 0.02, round down to three decimals, and clamp to [0, 0.5]. Freeze that
number before testing the held-out set or the no-answer queries. Keep all original,
held-out and no-answer acceptance requirements above unchanged. This uses the
original set for calibration; it does not relabel it as held-out acceptance.
