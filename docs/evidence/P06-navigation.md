# P06 — Native sidebar routes, thread originals and paging

Status: bounded current-stack implementation; **P06 and the full client remain
incomplete**. Everything below uses synthetic accounts and mail.

## Behavior

Kit's `Sidebar` routes the unified inbox, each account and each known cached folder
through the existing store query, without starting a sync. Mailbox and thread lists
keep one 100-row keyset page; Newer/Older retain only cursor history. Expanding a
thread's message count (`Collapsible` + Kit `List`) reads metadata only: arrows
select passively and Enter or a click opens exactly that original. `DescriptionList`
shows parsed From/To/Cc and sent/received dates in the declared timezone (UTC when
unset). Changing scope, query or page retires the previous original and its reader.

A local read intention now targets every current folder copy of the opened original,
so opening a cached Sent or Archive message can be recorded. Unread siblings and other
accounts acquire no intention, and each UID stays bound to its folder and UIDVALIDITY.

Two defects were found and fixed while validating this slice:

- A focused control leaving the tree (a pager button becoming disabled, a retired
  thread list) emptied the focus path, so global key bindings stopped working. The
  workspace now reclaims focus through GPUI's `on_focus_lost`.
- The read-only `Textarea` drew each carriage return in CRLF mail as a missing glyph.
  Only the displayed copy is normalized; the cached original is unchanged.

## Checks

Run on the CI server through `ci/remote.sh`, not the laptop. `bash ci/check.sh` in
the pinned `nix develop` shell passes formatting, tool lints, strict Clippy and the
normal workspace tests.

`MAILSTRONAUT_MAILBOX_NAVIGATION=1 experiments/p06-mailbox/wayland.sh` passes in a
private headless sway session with `DISPLAY` unset, a software Vulkan driver and a
private bus. It drives real keyboard and pointer input and asserts SQLite Seen state
after each step: passive selection, thread expansion and paging; one original marked
per explicit open; Older followed by Ctrl+L/Down/Enter; refresh retaining the page;
Sent and other-account routes; a declaratively disabled account retiring its open
message while keeping its cache; and an offline restart.

## Not established

No live provider, real desktop session, assistive-technology, large-mailbox timing or
Flatpak acceptance. Unread `Badge::count` on sidebar rows is [verified separately](P04-background-sync.md), and folder-specific
sync is not implemented, so a folder lists only what is already cached.

## Message actions

The reader toolbar adds Reply, Reply all, Forward, Mark read/unread (Alt+U) and Star
(Alt+S). `MAILSTRONAUT_MAILBOX_NAVIGATION=1` now also opens an original, stars it, marks
it unread and replies. SQLite shows it unread and starred, with an unsent local draft
carrying `In-Reply-To`, the sender as recipient and a quotation. Later refreshes do not
mark it read again, because the read gate is spent by the explicit open.

## Acting on an older original

The private Wayland fixture at `eec2fcc` expanded a three-message conversation,
selected the older reply, and displayed its actual From, To, Cc, Sent and Received
fields. [Header details for the selected sibling](artifacts/p06-original-actions/headers.png).
It starred the older reply, marked it unread and replied. SQLite confirmed that
only the selected original was starred. The newer sibling stayed read. The draft
was addressed to the older sender, quoted only the older body and contained its
exact `In-Reply-To`. The outbox remained empty.

Run the standard private mailbox walkthrough with `MAILSTRONAUT_MAILBOX_NAVIGATION=1`
and `MAILSTRONAUT_ORIGINAL_ACTIONS_GUI=1`. The unmodified navigation walkthrough
and the long-thread walkthrough also passed on the same pinned build. They covered
the cached Sent parent, a matching original in another account, an offline restart,
passive keyboard traversal over a page boundary and explicit opening in a thread
of 105 originals. [Unsent reply to the older message](artifacts/p06-original-actions/reply.png).
