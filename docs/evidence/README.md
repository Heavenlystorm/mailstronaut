# Evidence scope

The current architecture is GPUI Kit + Blitz, with native Wayland required.
`docs/PLAN.md` is the authoritative current roadmap.

Evidence from the previous UI/browser architecture records historical results at
its stated revision. Its screenshots, window interaction, renderer/privacy/theme,
accessibility and GUI package claims are **not** acceptance of the new stack.
Those checklist rows were reset or replaced rather than carried forward.

Retained configuration, storage, MIME, protocol, OAuth and credential tests remain
useful backend evidence. They must pass again with the new dependency graph;
backend proof alone does not establish GPUI or Blitz behavior. New foundation
results are recorded in `P01-gpui-blitz.md`, with precise limitations.
