# Interface direction and component reuse

The product is a minimal, dependable daily email client. Prefer a small amount of
application code connecting maintained components to the existing mail engine.
[PLAN.md](PLAN.md) owns implementation status; this document describes the design
and review approach, not completed UI acceptance.

## Start with GPUI Kit

Before implementing a view or interaction, ask whether a pinned GPUI Kit component
already solves it. Use its layout, semantic theme, focus, disabled states and
interaction behavior. Do not wrap every component in an application abstraction
or rebuild one to reproduce a screenshot. A small amount of layout/composition
code is expected; replacing an existing component requires a concrete limitation.

The following APIs were checked against the project's GPUI Kit/component 0.8.1
source and the matching official documentation on 2026-10-31:

| Mail workflow | Reuse | Application responsibility |
| --- | --- | --- |
| Unified inbox, account/folder navigation, Settings entry | `Sidebar::new(id)`, `SidebarHeader`, `SidebarGroup`, `SidebarMenu`, `SidebarMenuItem`, `SidebarFooter` | Stable account/folder identities, route selection, bounded counts; no empty feature destinations |
| Mailbox/list/reader layout | Existing resizable panels and GPUI virtualized lists | Paged snapshots, stable row identity/scroll, cancellation, distinct focus/selection/Seen semantics |
| Preferences and account configuration | `Settings::new(id)`, `SettingPage`, `SettingGroup`, `SettingItem`, `SettingField` | Native Nix ownership, validation, reload/save/export; existing page/search layout instead of custom settings navigation |
| Authentication | Masked `Input`, `Button` and standard status controls within Settings | Foreground encrypted-keyring access and external-browser OAuth; never store secrets in a `SettingField` or configuration export |
| Draft fields and readable text | Standard inputs and multiline editor/read-only components | Identity, recipient validation, autosave and immutable send state; no bespoke text editor |
| Received and composed attachments | `Attachment`, `AttachmentGroup`, typed media/content/title/description/action slots | Bounded owned bytes, real lifecycle, safe portal actions and private previews |
| Progress, failures, menus and confirmations | Existing buttons, spinners/progress, alerts, menus, dialogs and tooltips | Honest operation state, cancellation/retry/undo and useful recovery; no decorative placeholder actions |

Sources: [Sidebar](https://gpui-kit.com/component/sidebar/),
[Settings](https://gpui-kit.com/component/settings/),
[Attachment](https://gpui-kit.com/component/attachment/).
Use the published `gpui_kit` facade/reexports already selected by the workspace;
do not introduce a second framework or component version.

Settings should have only pages needed by implemented capabilities. Reuse built-in
setting fields and disabled states; compound authentication controls can use the
custom-element slot. This is not a reason to reproduce page/search/group layout
inside another custom entity. Account actions remain possible when the account's
declaration is managed, because passwords and refresh tokens live in the keyring.

The Attachment component supplies presentation and remove/retry callbacks, not
file ownership, transport or safe decoding. Use filename, type and size with a
generic file icon first. Optional thumbnails must come from validated, bounded
owned raster bytes, never an email URL or a sender-provided filesystem path.
GPUI image sources are outside Blitz's resource provider; that provider cannot
protect a remote URL passed directly to an attachment thumbnail. Received Save/
Open and composer Remove are different actions. Do not display an upload animation
for a file that was only copied locally into a draft, or imply independent uploads
to a service. Support loading/processing/failure/ready states and keyboard actions
without inventing an attachment viewer or document-conversion pipeline.

## Daily-use review

Review the actual application at completed workflow boundaries, not just once at
release. Ask: could someone do their daily mail here, and would they choose it?
Use these concrete checks:

- Current account/folder, unread state and selected original are clear. Unified
  rows identify their account; priority and deadlines add information without
  hiding normal mail. Focus, selection and read state remain distinct.
- Sender, subject, date and an extracted two/three-line preview are easy to scan.
  Long names, Unicode, attachments and thread counts do not displace essential
  information or require a forest of icons.
- Search and compose are easy to find. Searching, opening a matching original,
  moving through a thread, replying and handling an attachment require little
  navigation. Retain reader space; reveal secondary actions when appropriate.
- Empty, loading, partial-cache, offline, authentication and failed-send states
  explain what happened and offer a real next action. Cached mail stays readable;
  a spinner is not an indefinite or silently failed state.
- Keyboard-only use, resize, text scaling, light/dark themes and large-list
  responsiveness hold up in an actual native Wayland session. Measure performance;
  framework choice and screenshots alone are not performance evidence.
- Home Manager users can understand ownership, reload their native declaration,
  optionally edit mutable preferences and export a reviewable Nix snapshot.
- Each control serves an implemented email requirement. Do not import calendars,
  chat, meetings, collaboration, marketing categories, or general productivity
  suites because they appear in another client's screenshot.

After each implemented feature, capture and show the current application in chat.
Use the private Wayland test profile and synthetic accounts/mail, without touching
the user's mailbox or desktop settings. Show relevant theme/size/error variants
when changed. Identify the actual build and unverified behavior; an attractive
screenshot is not proof of networking, privacy, accessibility or durable state.
This applies to backend features too: show the current UI and state honestly if
the feature has no visible control yet. Reference images and mockups never stand
in for an actual application capture.

## Supplied reference images

These are usability references, not a style specification, feature backlog or
licensed application assets. All seven were downloaded and inspected on
2026-10-31. Local copies and their URL/hash/dimension manifest are in ignored
`target/design-references/`; never commit or distribute the images.

| Reference | Useful review question |
| --- | --- |
| [Apple Mail reader](https://ipcdn-web.apple.com/assets/v2/web/ca4c7412-b7a7-4282-894b-dc6a91eef72c) | Can the user scan a conversation list while the reader retains useful space? |
| [Apple Mail search](https://ipcdn-web.apple.com/assets/v2/web/d297f49f-45f7-4f94-8010-8fb45b41626c) | Are search scope and results clear without obscuring normal navigation? |
| [Apple Mail composer](https://ipcdn-web.apple.com/assets/v2/web/b4f1aa05-bc20-45b3-98b1-d6ac42d2597e) | Are recipients, identity, subject, content and sending understandable? |
| [Thunderbird](https://blog.thunderbird.net/files/2023/07/115.png) | Can multiple accounts/folders and essential actions be found without excessive interface density? |
| [Spark](https://media.idownloadblog.com/wp-content/uploads/2016/11/Spark-for-macOS-Mac-screenshot-002.jpg) | Do thread structure and attachment metadata remain readable? |
| [Proton Mail](https://i.pcmag.com/imagery/articles/04f5OWqxR498kT6yaSfpidA-1.fit_lim.v1710419330.png) | Are dark-mode hierarchy, active navigation and message actions clear? |
| [Gmail](https://techcrunch.com/wp-content/uploads/2022/07/Gmail_GM3.max-1000x1000-1.png) | Can a dense mailbox still be scanned quickly with prominent search? |

Keep GPUI Kit's design language. Do not reproduce these clients' colors, layouts,
category systems or incidental functionality. Record actionable shortcomings in
the relevant PLAN milestone and keep acceptance unchecked until corrected.
