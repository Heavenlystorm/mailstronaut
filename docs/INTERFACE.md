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

Use the published `gpui_kit` facade/reexports already selected by the workspace;
do not introduce a second framework or component version.

The requirement-by-requirement mapping below incorporates the requested read-only
catalog audit. It selects components, not new features. PLAN rows link here for
API details and retain their own behavioral acceptance. Backend-only requirements
such as UID reconciliation, model inference or Nix activation have no widget
substitute. Prefer the styled Component layer; [Base](https://gpui-kit.com/base/)
is appropriate only for a missing primitive, not for recreating a design system.

### P01: reader and shell

| Subproblem | Component/API | Boundary |
| --- | --- | --- |
| Native window/theme | `Root`, `TitleBar`, `Theme::sync_system_appearance`/`change` | Keep native Wayland selection and actual platform/theme tests. |
| HTML/text mode | [`TabBar::new(id).segmented()` and `Tab`](https://gpui-kit.com/component/tabs/) | Switch the existing Blitz and native text views; do not replace Blitz with the catalog's HTML/Markdown `TextView`. |
| Read-only selectable native text | [`TextareaState`, `Textarea::readonly(true)`](https://gpui-kit.com/component/textarea/) | Decoded literal text, selection/copy and keyboard scrolling; not a custom editor. |
| Reader actions/scrollbars | [`Toolbar`/`ToolbarGroup`](https://gpui-kit.com/component/toolbar/), [`ScrollableElement`, `vertical_scrollbar`, GPUI `ScrollHandle`](https://gpui-kit.com/component/scrollable/) | These do not implement viewport rasterization, HTML hit testing, selection, generations or accessibility. |
| Explicit external links | `Button` invoking validated ASHPD portal action; optionally `Link` **without `href`** | Do not use default URL navigation for untrusted mail. See pin caveats below. |

### P02: settings and P05: onboarding

| Subproblem | Component/API | Boundary |
| --- | --- | --- |
| Preferences pages, search and fields | [`Settings`, `SettingPage`, `SettingGroup`, `SettingItem`, `SettingField::{input,switch,dropdown,scrollable_dropdown,number_input,element,render}`](https://gpui-kit.com/component/settings/) | Reuse navigation/search/group layout. Only implemented settings; custom compound slots honor `RenderOptions::is_disabled()`. |
| Native source path/ownership/reload state | [`DescriptionList`](https://gpui-kit.com/component/description-list/), [`Clipboard::value/value_fn`](https://gpui-kit.com/component/clipboard/), [`Alert`](https://gpui-kit.com/component/alert/), Reload `Button` | Keep long paths inspectable/copyable; a disabled narrow input is not sufficient. Managed declaration stays read-only. |
| Setup/manual account forms | [`Form`/`Field::{label,description,required,visible}`](https://gpui-kit.com/component/form/), [`Select`/`SelectState`](https://gpui-kit.com/component/select/), `Input`, [`NumberInput`](https://gpui-kit.com/component/number-input/) | Provider/auth/TLS choices, address/login/host and ports; typed validation and secure transport remain authoritative. |
| Optional SMTP/folder/CA options | [`Collapsible`](https://gpui-kit.com/component/collapsible/); [`InputGroup`, read-only `Input`, `InputGroupButton`](https://gpui-kit.com/component/input-group/) for Browse/Import | Reveal advanced options when needed; the button invokes the file portal, not ambient filesystem access. No new wizard required. |
| Password and OAuth/keyring actions | `InputState::masked(true)`, `Input`, explicit `mask_toggle`, Save/Sign in/Unlock/Reauthenticate/Cancel `Button`s, `Spinner`, `Alert` | Foreground encrypted storage/external browser; no serializable secret setting, custom unlock dialog or credential clipboard action. |
| Export to Nix | [`Dialog`](https://gpui-kit.com/component/dialog/), read-only `Textarea`, `Clipboard`, Save `Button`, in-app `Notification` | Prepare and validate native text off the UI thread. Saving uses the portal; do not add a code editor, file browser or dotfile rewriter. |

Settings should have only pages needed by implemented capabilities. Account
credential actions remain available when declarations are managed, because
passwords and refresh tokens live in the keyring. Keep persistent input entities
outside dialog/sheet render closures: builders can run again, so rendering must
not recreate/focus fields or trigger network work.

### P03: store and P04: synchronization

| Subproblem | Component/API | Boundary |
| --- | --- | --- |
| Empty/unavailable/corrupt store | [`Empty`, `EmptyHeader/Title/Description/Content`](https://gpui-kit.com/component/empty/), persistent `Alert`, Retry `Button` | Distinguish empty from failure; [`AlertDialog`](https://gpui-kit.com/component/alert-dialog/) only for an explicitly destructive decision. |
| Sync/rebuild progress and cancellation | `Button::loading(true)` or [`Spinner`](https://gpui-kit.com/component/spinner/), Cancel/Retry `Button`s; [`Progress`](https://gpui-kit.com/component/progress/) only with a measured total | Present actual engine state without a custom animation or invented percentage. |
| Account errors, offline and partial coverage | `Alert`, [`StatusBar::left/right`](https://gpui-kit.com/component/status-bar/) | Keep useful cached mail visible and errors recoverable. |
| Older paging | `ListDelegate::has_more/load_more` or explicit Load older `Button` | Backend cursor/UID scope remains authoritative; no whole-mailbox loading. |

### P06: mailbox, threads and search

| Subproblem | Component/API | Boundary |
| --- | --- | --- |
| Unified/account/folder navigation | [`Sidebar::new(id)`, `SidebarHeader/Group/Menu/MenuItem/Footer`](https://gpui-kit.com/component/sidebar/), `Badge::count` | Use Sidebar nesting first. Consider [`Tree`/`TreeState`](https://gpui-kit.com/component/tree/) only for a demonstrated large hierarchical folder requirement. |
| Split panes | [`h_resizable`, `resizable_panel`, `ResizableState`](https://gpui-kit.com/component/resizable/) | Ordinary mail splits, not a Dock/tabbed workspace framework. |
| Virtualized equal-height rows, sections and paging | [`List`, `ListState`, `ListDelegate`, `ListItem`](https://gpui-kit.com/component/list/); `ListState::searchable(false)` | One external search field. Stable canonical identities and paged snapshots remain app state. For necessary variable-height rows use existing `v_virtual_list`/`VirtualListScrollHandle`, not custom virtualization. |
| Selection versus explicit opening | `ListDelegate::set_selected_index` versus `confirm`; `ListEvent::Select/Confirm` | Selection is passive. Route confirmation once to the existing content-load/identity/generation gate before Seen; do not invoke opening from both delegate and event handlers. |
| Sender/subject/date/preview and account/unread/thread indicators | [`Label`](https://gpui-kit.com/component/label/), `Icon`, [`Badge::count/dot`](https://gpui-kit.com/component/badge/), `ListItem::accessibility_label` | Compose text hierarchy in standard rows; accessible labels are explicit. Optional Avatar uses initials/local data, never remote avatar services. |
| One query field, clear and optional filters | `InputGroup`/`Input`/`InputGroupButton`, [`Popover`](https://gpui-kit.com/component/popover/), `Select`/[`Combobox`](https://gpui-kit.com/component/combobox/), sender `Input`, `DatePickerState::range` | The maintained local search engine owns ranking/filter semantics, not component substring search. |
| Match snippets | `Label::highlights` for simple literal matching, GPUI `StyledText::with_highlights` for backend UTF-8 ranges | Do not invent literal matches for semantic hits. A code-syntax highlighter is unnecessary. |
| Thread originals and headers | `Collapsible`, or [`Accordion::multiple(true)`](https://gpui-kit.com/component/accordion/) if its toggle model fits; `DescriptionList` | Retain original-specific opening and state; avoid chat bubbles and automatic tail following. |
| Loading/no results/coverage/errors | List `render_empty`, `loading`, `render_loading`; `Empty`, [`Skeleton`](https://gpui-kit.com/component/skeleton/), `Alert`, `StatusBar` | Missing semantic model, partial index and offline cache are distinct. Stop hidden loading animations. |

### P07: composition and actions

| Subproblem | Component/API | Boundary |
| --- | --- | --- |
| Identity, recipients, subject, body/signature | `Form`/`Field`, identity `Select`, `Input`, `Textarea` | Lettre validates addresses/builds MIME; no custom text editor or source-code Editor/LSP/gutters. |
| Cc/Bcc and optional recipient chips | `Collapsible`; plain `Input` first; existing `InlineToken`/`InputToken` and `replace_with_token`/`replace_range_with_token` when needed | Reuse atomic token editing. `Combobox::multiple(true)` is a finite-choice picker, not arbitrary-email entry or a reason to add contacts. |
| Received/composer attachments | [`Attachment`, `AttachmentGroup`, `AttachmentMedia/Content/Title/Description/Actions`](https://gpui-kit.com/component/attachment/) | Real owned bytes/status, portal actions and safe local previews; see resource policy below. |
| Send/reply/forward/read/star/archive/trash/move | `Toolbar`/`ToolbarGroup`, `Button`, [`DropdownButton`](https://gpui-kit.com/component/dropdown-button/), [`PopupMenu`/`ContextMenuExt`](https://gpui-kit.com/component/menu/), searchable destination `Combobox` | Reuse shared sizing/roving focus and menu behavior; UID-safe backend actions remain unchanged. |
| Draft/save/outbox/error/Undo | `StatusBar`, persistent `Alert`, [`Notification::action` in explicit InApp mode](https://gpui-kit.com/component/notification/) | A toast does not own durable state. Unknown delivery is not a generic retry case. |
| Discard/purge/ambiguous recovery | `AlertDialog`, or `Dialog` for a contextual choice | Bind decisions to exact durable state; no extra confirmation for routine reversible operations. |

### P08: reminders and background

| Subproblem | Component/API | Boundary |
| --- | --- | --- |
| Presets and custom date/time | `DropdownButton`/`PopupMenu`; [`DatePicker`, `DatePickerState::time_precision(TimePrecision::Minute)`, `hour_cycle`, `default_time`, `DateRangePreset::date_time`](https://gpui-kit.com/component/date-picker/) | The date picker already embeds time input. Chrono TZ resolves its civil time and persists UTC; component selection is not scheduling/DST policy. |
| Morning/evening defaults, timezone, reschedule/cancel | [`TimeField`](https://gpui-kit.com/component/time-field/), searchable timezone `Combobox`, `DescriptionList`, Save/Cancel `Button`s | Keep generation and ambiguous/nonexistent local-time checks in the backend. |
| Background/autostart/notification preferences | `SettingField::switch` with ownership/disabled text | Independent preferences; a switch is not portal permission or control of HM-managed services. |
| Foreground feedback versus background reminders | GPUI `Notification` explicitly InApp for foreground status | The pinned system path cannot provide the durable reminder actions; retain existing stable IDs/portal/backend route. |

### P09: priority, deadlines and models

| Subproblem | Component/API | Boundary |
| --- | --- | --- |
| Priority grouping/highlight/overflow | List sections/headers, `Label`, `Icon`, `Badge::count`, semantic surfaces, Show all `Button` | Backend eligibility/expiry/order, one mailbox list; no new grouping engine or competing scroll area. |
| Deadline box/evidence/correction | [`GroupBox`](https://gpui-kit.com/component/group-box/), `Label`/`Icon`; clicked `Popover` or reader `DescriptionList`; `Dialog`/`Form`/`DatePicker`, Dismiss `Button` | Box below extracted preview, evidence available without hover, persisted correction/dismissal. The widget never infers a date or revives priority. |
| Local model enablement/import/status | `SettingField::switch`, metadata `DescriptionList`, Import `Button`/portal, real `Progress`/`Spinner`, `Alert` | Explicit hashed installation only; no marketplace, remote endpoint, silent download or confidence dashboard. |

### P10/P11: integration and usability acceptance

Reuse P02 Settings/source/export and P05 file-import components for native and
Flatpak configurations. Nix/HM reconciliation, package closure, sandbox permissions
and background delivery are backend requirements, not separate UI frameworks.

Use [`Kbd::binding_for_action`](https://gpui-kit.com/component/kbd/) and Tooltip for
actual bound shortcuts, and standard Toolbar/Menu/Dialog/Select/List focus
behavior. Use `FocusTrapElement::focus_trap` only for a genuinely custom overlay;
do not duplicate built-in modal behavior. Real Wayland keyboard/AT-SPI/screen-reader
proof remains mandatory. Component APIs and aria hooks alone are not accessibility
acceptance.

### Pin-specific integration limits

- `SidebarMenu`/`SidebarMenuItem` implement `SidebarItem`, not ordinary
  `IntoElement`. A footer can use a normal Settings `Button`, or explicitly render
  the item through that trait; `SidebarFooter::child(SidebarMenu)` does not compile.
- [`Link` source](https://docs.rs/crate/gpui-component/0.8.1/source/src/link.rs)
  calls `cx.open_url(href)` before `on_click`; `disabled` is historically inert.
  Use validated portal callbacks on `Button` or a `Link` with no `href`.
- [`List` source](https://docs.rs/crate/gpui-component/0.8.1/source/src/list/list.rs)
  defaults `searchable` to false. `SearchableListItem` defaults to case-insensitive
  substring matching, and `SearchableVec` filters an in-memory vector. They suit
  small option menus, not mailbox fuzzy/multilingual retrieval.
- [`Notification` source](https://docs.rs/crate/gpui-component/0.8.1/source/src/notification.rs)
  submits system notifications with `actions: Vec::new()` and window-bound dispatch.
  Its tag uses a process/binary type identity, and Linux retraction is documented
  unsupported. Do not substitute it for durable reminder IDs/actions/background work.

Do not add Dock, a command palette, chat MessageScroller, charts, ratings, color
editors, speech, calendar/contacts products or extra preferences merely because
the catalog has them. `Command`/`CommandItem` is an existing option only if a command
palette becomes a justified future requirement.

## Attachment resource policy

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
