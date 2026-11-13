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
