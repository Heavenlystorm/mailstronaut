# P09 — Priority rules and deadlines, without a model

Status: native acceptance of the **rules baseline** against a real server with synthetic
mail. No model is selected or installed, and this does not complete P09. CLASSIFIER.md's
held-out acceptance experiment has not been run.

## Behavior

With `priority.enabled = true`, unread inbox mail up to 30 days old is analyzed in the
background, without opening it or setting Seen. A message joins the **Needs attention** group
above the unified inbox when it states a deadline and is either addressed to the reader or
asks for something, or when it is addressed to the reader and asks for something. Mailing
footers and "no action required" in English, French or Italian rule it out, whatever else
it says. The group is ordered by what ends soonest, and a conversation appears once.

A deadline needs a cue attached to the date ("submit by", "date limite", "entro"). Meetings,
flights, invoice dates, marketing, deliveries, past dates, quoted text and signatures do
not qualify, and neither does a negated cue. Writing that cannot be resolved with certainty
is kept but never acted on: `03/04/2027`, "by Friday" sent on a Friday, an abbreviation such
as CST, or a time inside a clock change.

A date-only deadline ends at the next local midnight, however long that day is. An explicit
time ends at that instant. Without a deadline, priority lasts 72 hours from receipt, or 24
if the text says it is urgent, capped by `priority.maxTtlHours`. The end is fixed at analysis
and stored, so it cannot slide. Relative dates use the Date header, unless it is missing or
more than 48 hours from receipt, in which case receipt is used and the assumption recorded.

Reading or dismissing is final. A message marked read once stays out even if marked unread
again, and a later result cannot bring it back. Dismissing does not read it.

## Design

`crates/mailstronaut-priority` is pure: no model, no network and no clock, so the same message
always gives the same versioned result, with reasons and exact byte spans for its evidence.
The store owns eligibility in `priority_items`, and a stale result cannot override it. Schema
11 extends the `analysis` table. `mail_priority.rs` feeds it batches of 64 in the background.

## Checks

`experiments/p09-priority/gui.sh`, called by `credentials.sh` with
`MAILSTRONAUT_PRIORITY_GUI_BINARY`, runs against a real Dovecot in a private sway session:

- Of six messages, only a French deadline and an Italian question qualify. The newsletter that
  also asks for a reply, the flight, the meeting notes and mail to a list stay out.
- Analysis sets no Seen flag.
- Dismissing with the row's button removes it without reading it. This run found the click also
  reaching the row underneath, which opened the message, and that is fixed.
- Opening the other message removes it, and marking it unread does not bring it back.
- A delivery arriving while running joins the group with no input.
- After a restart, results and dismissals are unchanged and nothing is analyzed twice.

`crates/mailstronaut-priority/tests/deadlines.rs` covers wording in three languages, negation,
quoting, year rollover, leap days, ambiguity, 23- and 25-hour days, a midnight that does not
exist (Havana), times inside clock changes, delayed delivery, forged Date headers, several
dates in one message, unsupported languages, instructions embedded in mail and hostile input.
`store::analysis` covers eligibility, final reads and dismissals, stale results and moves.

`tests/corpus.rs` runs 60 labelled synthetic messages, 20 in each language:

| | Priority precision | Priority recall | Deadline precision |
| --- | --- | --- | --- |
| English | 1.00 | 0.90 | 1.00 |
| French | 1.00 | 1.00 | 1.00 |
| Italian | 1.00 | 1.00 | 1.00 |

**These numbers flatter the rules.** The messages were written alongside them, so this guards
against regressions and says little about real mail. CLASSIFIER.md asks for at least 180
messages split into development and held-out sets, which has not been done.

## Not established

Any model, and therefore semantic judgment: the rules only see stated cues. Real mail. Showing
why a message qualified, correcting a date, Show all, and overflow past 20 rows. The Settings
switch: the feature is off unless `priority.enabled = true` is declared. Time zones beyond
the configured or system zone. Phrasings outside the tables simply are not recognized.
