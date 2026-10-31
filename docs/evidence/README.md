# Evidence scope

The current architecture is GPUI Kit + Blitz, with native Wayland required.
`docs/PLAN.md` is the authoritative current roadmap.

Evidence from the previous UI/browser architecture records historical results at
its stated revision. Its screenshots, window interaction, renderer/privacy/theme,
accessibility and GUI package claims are **not** acceptance of the new stack.
Those checklist rows were reset or replaced rather than carried forward.

The cached-original HTML connector, native viewport/modes and current acceptance
are recorded in [P01-cached-html.md](P01-cached-html.md). P01 remains open. The
preceding passive-resource syscall observations, retained viewport API and MIME
raster extraction are recorded in [P01-passive-viewport.md](P01-passive-viewport.md).

Retained configuration, storage, MIME, protocol, OAuth and credential tests remain
useful backend evidence. They must pass again with the new dependency graph;
backend proof alone does not establish GPUI or Blitz behavior. New foundation
results are recorded in `P01-gpui-blitz.md`, with precise limitations.

Current GPUI account/settings, encrypted password storage, explicit TLS/PEEK sync,
Seen and offline-restart results are in [P02-gpui-accounts.md](P02-gpui-accounts.md).
The older P02 account UI walkthrough remains historical.

Current GPUI mailbox List, passive selection, explicit confirmation, overlapping
flag/query/body work and canonical unread repaint results are in
[P06-gpui-mailbox.md](P06-gpui-mailbox.md). Earlier X11 mailbox walkthroughs remain
historical frontend evidence.
