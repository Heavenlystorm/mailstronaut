//! Standard GPUI Kit list; selection never authorizes reading a message.

use std::{collections::HashSet, rc::Rc};

use gpui::base::StyledExt as _;
use gpui::component::{
    ActiveTheme as _, IconName, IndexPath, Sizable as _,
    badge::Badge,
    button::{Button, ButtonVariants as _},
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyTitle},
    label::Label,
    list::{ListDelegate, ListItem, ListState},
    tag::Tag,
};
use gpui::{
    App, Context, Hsla, IntoElement, ParentElement as _, Pixels, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};

const ROW_HEIGHT: Pixels = px(124.);
const SECTION_HEIGHT: Pixels = px(28.);

#[derive(Clone)]
pub struct Row {
    pub account: String,
    pub id: i64,
    pub conversation: Option<i64>,
    pub sender: String,
    pub subject: String,
    pub preview: String,
    pub unread: u32,
    pub messages: u32,
    pub received_at: i64,
    /// This original's own flags, separate from the group's unread count.
    pub seen: bool,
    pub starred: bool,
    /// The deadline that keeps it in the priority group, as an ISO local date and time.
    pub due: Option<String>,
    /// When it leaves the priority group, if it is in it.
    pub expires_at: Option<i64>,
}

impl From<mailstronaut_core::store::conversations::Summary> for Row {
    fn from(row: mailstronaut_core::store::conversations::Summary) -> Self {
        Self {
            unread: row.unread,
            conversation: row.conversation,
            messages: row.cached_messages,
            account: row.latest.account,
            id: row.latest.id,
            sender: row.latest.sender,
            subject: row.latest.subject,
            preview: row.latest.preview,
            received_at: row.latest.received_at,
            seen: row.latest.seen,
            starred: row.latest.starred,
            due: None,
            expires_at: None,
        }
    }
}

impl Row {
    pub fn same_group(&self, other: &Self) -> bool {
        self.account == other.account
            && match (self.conversation, other.conversation) {
                (Some(a), Some(b)) => a == b,
                _ => self.id == other.id,
            }
    }
}

/// Internal metadata cursor; batches never become user-facing pages.
#[derive(Default)]
pub struct Browse {
    pub after: Option<mailstronaut_core::store::mailbox::Cursor>,
    pub loaded: usize,
    pub more: bool,
    pub pending: bool,
}

/// The declared timezone, else the system's, else UTC. Civil-time features such as
/// "tomorrow morning" need a real zone even when the configuration leaves it out.
pub fn local_zone(declared: Option<&str>) -> String {
    declared
        .map(str::to_owned)
        .or_else(|| iana_time_zone::get_timezone().ok())
        .filter(|zone| zone.parse::<chrono_tz::Tz>().is_ok())
        .unwrap_or_else(|| "UTC".into())
}

/// Native display follows the declared timezone; date formatting stays in Chrono.
pub fn date(timestamp: i64, zone: &str) -> String {
    let Some(date) = chrono::DateTime::from_timestamp(timestamp, 0) else {
        return "Date unavailable".into();
    };
    let zone = zone.parse::<chrono_tz::Tz>().unwrap_or(chrono_tz::UTC);
    date.with_timezone(&zone)
        .format("%d %b %Y · %H:%M %Z")
        .to_string()
}

/// Kit's text input draws a stray carriage return as a missing glyph, and MIME text
/// is normally CRLF. This changes only the displayed copy, never the cached original.
pub fn display_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// A stored deadline as a person says it, relative to today in the same zone.
pub fn due_label(due: &str, today: chrono::NaiveDate) -> String {
    let (date, time) = due
        .split_once('T')
        .map_or((due, None), |(date, time)| (date, Some(time)));
    let Ok(date) = date.parse::<chrono::NaiveDate>() else {
        return "Has a deadline".into();
    };
    let day = match (date - today).num_days() {
        0 => "today".to_owned(),
        1 => "tomorrow".to_owned(),
        _ => date.format("%-d %b").to_string(),
    };
    time.map_or_else(|| format!("Due {day}"), |time| format!("Due {day}, {time}"))
}

pub fn addresses(addresses: &[mailstronaut_core::mime::Mailbox]) -> String {
    if addresses.is_empty() {
        return "—".into();
    }
    addresses
        .iter()
        .map(|mailbox| match (&mailbox.name, &mailbox.address) {
            (Some(name), Some(address)) => format!("{name} <{address}>"),
            (Some(name), None) => name.clone(),
            (None, Some(address)) => address.clone(),
            (None, None) => "—".into(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub type Dismiss = Rc<dyn Fn(i64, &mut Window, &mut App)>;
/// Called when the user asks to see every conversation that needs attention.
pub type ShowAll = Rc<dyn Fn(&mut Window, &mut App)>;

/// The priority group as it is shown, and whether more conversations need attention.
#[derive(Default)]
pub struct Priority {
    pub rows: Vec<Row>,
    pub more: bool,
}

#[derive(Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Independent Kit loading, search, expansion and continuation flags are not mutually exclusive states"
)]
pub struct Rows {
    pub items: Vec<Row>,
    /// The priority group, shown above the mailbox only when it has something in it.
    pub priority: Priority,
    /// Whether the user asked for every conversation that needs attention.
    pub show_all_priority: bool,
    pub loading: bool,
    pub searching: bool,
    pub time_zone: String,
    /// Called with a row's message when the user dismisses it from the priority group.
    pub on_dismiss: Option<Dismiss>,
    /// Called when the user asks for every conversation that needs attention.
    pub on_show_all: Option<ShowAll>,
    /// Kit calls this near the end of the virtualized list.
    pub on_load_more: Option<ShowAll>,
    pub more: bool,
}

impl Rows {
    /// Append metadata without duplicating canonical conversations or ungrouped originals.
    pub fn append(&mut self, rows: Vec<Row>) {
        let existing = self.items.iter().chain(&self.priority.rows);
        let mut groups = HashSet::new();
        let mut ungrouped = HashSet::new();
        let mut messages = HashSet::new();
        for row in existing {
            messages.insert((row.account.clone(), row.id));
            if let Some(group) = row.conversation {
                groups.insert((row.account.clone(), group));
            } else {
                ungrouped.insert((row.account.clone(), row.id));
            }
        }
        for row in rows {
            let message = (row.account.clone(), row.id);
            let duplicate = row.conversation.map_or_else(
                || messages.contains(&message),
                |group| {
                    groups.contains(&(row.account.clone(), group)) || ungrouped.contains(&message)
                },
            );
            if duplicate {
                continue;
            }
            messages.insert(message.clone());
            if let Some(group) = row.conversation {
                groups.insert((row.account.clone(), group));
            } else {
                ungrouped.insert(message);
            }
            self.items.push(row);
        }
    }

    fn positions(&self) -> impl Iterator<Item = (&Row, Pixels)> {
        let header = if self.priority.rows.is_empty() {
            px(0.)
        } else {
            SECTION_HEIGHT
        };
        let mailbox = self
            .priority
            .rows
            .iter()
            .fold(header, |y, _| y + ROW_HEIGHT)
            + header;
        let positions = || {
            move |y: &mut Pixels, row| {
                let here = *y;
                *y += ROW_HEIGHT;
                Some((row, here))
            }
        };
        self.priority
            .rows
            .iter()
            .scan(header, positions())
            .chain(self.items.iter().scan(mailbox, positions()))
    }

    fn anchor(&self, offset: Pixels) -> Option<(Row, Pixels)> {
        if offset >= px(0.) {
            return None;
        }
        self.positions()
            .find(|(_, y)| *y + ROW_HEIGHT > -offset)
            .map(|(row, y)| (row.clone(), -offset - y))
    }

    fn anchored_offset(&self, anchor: &(Row, Pixels)) -> Option<Pixels> {
        self.positions()
            .find(|(row, _)| row.same_group(&anchor.0))
            .map(|(_, y)| -y - anchor.1)
    }

    /// The mailbox section comes second whenever the priority group is shown.
    const fn mailbox_section(&self) -> usize {
        if self.priority.rows.is_empty() { 0 } else { 1 }
    }

    pub fn row(&self, index: IndexPath) -> Option<&Row> {
        if index.section == self.mailbox_section() {
            self.items.get(index.row)
        } else {
            self.priority.rows.get(index.row)
        }
    }

    fn position(&self, row: &Row) -> Option<IndexPath> {
        let find = |rows: &[Row]| rows.iter().position(|other| other.same_group(row));
        find(&self.priority.rows).map(IndexPath::new).or_else(|| {
            find(&self.items).map(|found| IndexPath::new(found).section(self.mailbox_section()))
        })
    }
}

/// Cuts a priority group to `limit` rows unless `show_all` asks for all of it, and says
/// whether anything was cut. The store is asked for one row past the limit to tell that.
pub fn cut_group<T>(items: &mut Vec<T>, limit: usize, show_all: bool) -> bool {
    let more = !show_all && items.len() > limit;
    if more {
        items.truncate(limit);
    }
    more
}

/// Keep passive selection by canonical group, without scrolling or confirming it.
pub fn replace(
    list: &mut ListState<Rows>,
    rows: Vec<Row>,
    window: &mut Window,
    cx: &mut Context<ListState<Rows>>,
) {
    replace_with_priority(list, rows, Priority::default(), window, cx);
}

/// Like [`replace`], with a priority group. A conversation in the group is not repeated
/// in the mailbox below it.
pub fn replace_with_priority(
    list: &mut ListState<Rows>,
    mut rows: Vec<Row>,
    priority: Priority,
    window: &mut Window,
    cx: &mut Context<ListState<Rows>>,
) {
    let anchor = list.delegate().anchor(list.scroll_handle().offset().y);
    let old = list
        .selected_index()
        .and_then(|index| list.delegate().row(index).cloned());
    rows.retain(|row| !priority.rows.iter().any(|other| other.same_group(row)));
    list.delegate_mut().items = rows;
    list.delegate_mut().priority = priority;
    list.delegate_mut().loading = false;
    list.delegate_mut().more = false;
    let selected = old.and_then(|old| list.delegate().position(&old));
    list.set_selected_index(selected, window, cx);
    if let Some(y) = anchor.and_then(|anchor| list.delegate().anchored_offset(&anchor)) {
        let mut offset = list.scroll_handle().offset();
        offset.y = y;
        list.scroll_handle().set_offset(offset);
    }
    cx.notify();
}

impl ListDelegate for Rows {
    type Item = ListItem;

    fn sections_count(&self, _: &App) -> usize {
        self.mailbox_section() + 1
    }

    fn items_count(&self, section: usize, _: &App) -> usize {
        if section == self.mailbox_section() {
            self.items.len()
        } else {
            self.priority.rows.len()
        }
    }

    fn render_section_header(
        &mut self,
        section: usize,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<impl IntoElement> {
        if self.priority.rows.is_empty() {
            return None;
        }
        let (title, show_all) = if section == self.mailbox_section() {
            ("Everything else", None)
        } else {
            let show_all = self
                .priority
                .more
                .then(|| self.on_show_all.clone())
                .flatten();
            ("Needs attention", show_all)
        };
        Some(
            div()
                .h(SECTION_HEIGHT)
                .flex()
                .items_center()
                .justify_between()
                .px_4()
                .py_1()
                .border_b_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().muted)
                .child(Label::new(title).text_xs().font_semibold())
                .when_some(show_all, |view, show_all| {
                    view.child(
                        Button::new("show-all-priority")
                            .ghost()
                            .xsmall()
                            .label("Show all")
                            .on_click(move |_, window, cx| show_all(window, cx)),
                    )
                }),
        )
    }

    fn render_item(
        &mut self,
        index: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let row = self.row(index)?;
        let muted = cx.theme().muted_foreground;
        let today = mailbox_today(&self.time_zone);
        let due = row.due.as_deref().map(|due| due_label(due, today));
        let dismiss = (index.section != self.mailbox_section())
            .then(|| self.on_dismiss.clone())
            .flatten()
            .map(|dismiss| (dismiss, row.id));
        let unread = row.unread > 0;
        Some(
            ListItem::new(index)
                .h(ROW_HEIGHT)
                .px_4()
                .border_b_1()
                .border_color(cx.theme().border)
                .accessibility_label(format!(
                    "{} · {} · {} · {} messages · {} unread{}{}",
                    row.sender,
                    row.subject,
                    row.account,
                    row.messages,
                    row.unread,
                    if row.starred { " · starred" } else { "" },
                    due.as_deref()
                        .map_or_else(String::new, |due| format!(" · {due}"))
                ))
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(Label::new(row.sender.clone()).flex_1().truncate())
                                .when(row.starred, |view| {
                                    view.child(
                                        Label::new("★").text_color(cx.theme().primary).flex_none(),
                                    )
                                })
                                .child(div().w_3().h_3().flex_shrink_0().when(unread, |view| {
                                    view.child(
                                        Badge::new()
                                            .dot()
                                            .color(cx.theme().primary)
                                            .child(div().w_3().h_3()),
                                    )
                                }))
                                .when_some(dismiss, |view, (dismiss, id)| {
                                    view.child(
                                        Button::new(("dismiss-priority", index.row))
                                            .ghost()
                                            .xsmall()
                                            .icon(IconName::Close)
                                            .tooltip("Does not need attention")
                                            .on_click(move |_, window, cx| {
                                                // The row underneath must not also open,
                                                // since opening a message marks it read.
                                                cx.stop_propagation();
                                                dismiss(id, window, cx);
                                            }),
                                    )
                                }),
                        )
                        .child(
                            Label::new(row.subject.clone())
                                .truncate()
                                .map(|label| if unread { label.font_semibold() } else { label }),
                        )
                        .child(
                            Label::new(row.preview.clone())
                                .text_sm()
                                .text_color(muted)
                                .map(|label| {
                                    // Every row has the same height, so the deadline takes
                                    // the place of the preview's second line.
                                    if due.is_some() {
                                        label.truncate()
                                    } else {
                                        label.line_clamp(2)
                                    }
                                }),
                        )
                        .when_some(due, |view, due| {
                            view.child(div().flex().child(Tag::warning().small().child(due)))
                        })
                        .child(row_footer(row, &self.time_zone, muted)),
                ),
        )
    }

    fn set_selected_index(
        &mut self,
        _: Option<IndexPath>,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        // ListState owns the index and focus. Only ListEvent::Confirm opens mail.
        cx.notify();
    }

    fn has_more(&self, _: &App) -> bool {
        self.more && !self.loading
    }

    fn load_more(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        if self.more
            && !self.loading
            && let Some(load) = self.on_load_more.clone()
        {
            self.more = false;
            window.defer(cx, move |window, cx| load(window, cx));
        }
    }

    fn loading(&self, _: &App) -> bool {
        self.loading && self.items.is_empty()
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        Empty::new().header(
            EmptyHeader::new()
                .title(EmptyTitle::new().child(if self.searching {
                    "No cached matches"
                } else {
                    "No cached mail"
                }))
                .description(EmptyDescription::new().child(if self.searching {
                    "Try a different query. Only locally cached text is searched."
                } else {
                    "Open Settings to connect an account and sync its inbox."
                })),
        )
    }
}

fn row_footer(row: &Row, zone: &str, muted: Hsla) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .w_full()
        .child(
            Label::new(if row.messages > 1 {
                format!("{} · {} messages", row.account, row.messages)
            } else {
                row.account.clone()
            })
            .text_xs()
            .text_color(muted)
            .flex_1()
            .min_w_0()
            .truncate(),
        )
        .child(
            Label::new(date(row.received_at, zone))
                .text_xs()
                .text_color(muted)
                .flex_none(),
        )
}

fn mailbox_today(zone: &str) -> chrono::NaiveDate {
    let zone = zone.parse::<chrono_tz::Tz>().unwrap_or(chrono_tz::UTC);
    chrono::Utc::now().with_timezone(&zone).date_naive()
}

#[cfg(test)]
mod tests {
    use super::{Row, Rows, cut_group, date, display_text, due_label};
    use gpui::px;

    fn row(account: &str, id: i64, conversation: Option<i64>) -> Row {
        Row {
            account: account.into(),
            id,
            conversation,
            sender: String::new(),
            subject: String::new(),
            preview: String::new(),
            unread: 1,
            messages: 1,
            received_at: id,
            seen: false,
            starred: false,
            due: None,
            expires_at: None,
        }
    }

    #[test]
    fn appending_batches_preserves_prefix_and_scoped_canonical_identity() {
        let mut rows = Rows {
            items: vec![row("a", 1, Some(10)), row("a", 2, None)],
            ..Rows::default()
        };
        rows.priority.rows.push(row("a", 3, Some(20)));
        rows.append(vec![
            row("a", 4, Some(10)),
            row("a", 2, Some(30)),
            row("a", 5, Some(20)),
            row("a", 1, None),
            row("b", 1, Some(10)),
            row("a", 6, Some(40)),
            row("a", 7, Some(40)),
        ]);
        let ids = rows
            .items
            .iter()
            .map(|r| (r.account.as_str(), r.id))
            .collect::<Vec<_>>();
        assert_eq!(ids, [("a", 1), ("a", 2), ("b", 1), ("a", 6)]);
        assert_eq!(rows.priority.rows.len(), 1);
        assert!(rows.items.iter().all(|r| !r.seen));
    }

    #[test]
    fn a_refresh_keeps_the_visible_original_and_intra_row_offset() {
        let mut rows = Rows {
            items: (1..=10).map(|id| row("a", id, None)).collect(),
            ..Rows::default()
        };
        let anchor = rows.anchor(-px(260.)).expect("third row");
        assert_eq!(anchor.0.id, 3);
        rows.items.insert(0, row("a", 11, None));
        assert_eq!(rows.anchored_offset(&anchor), Some(-px(384.)));
        // A priority section adds two fixed header heights, without opening anything.
        rows.priority.rows.push(row("b", 12, None));
        assert_eq!(rows.anchored_offset(&anchor), Some(-px(564.)));
        assert!(
            rows.anchor(px(0.)).is_none(),
            "at the top newer mail should be visible"
        );
    }

    #[test]
    fn a_full_priority_group_shows_its_limit_and_says_more_remain() {
        let mut items: Vec<u32> = (0..21).collect();
        assert!(cut_group(&mut items, 20, false));
        assert_eq!(items.len(), 20);
        assert_eq!(items.last(), Some(&19));
    }

    #[test]
    fn a_group_of_exactly_the_limit_has_nothing_more() {
        let mut items: Vec<u32> = (0..20).collect();
        assert!(!cut_group(&mut items, 20, false));
        assert_eq!(items.len(), 20);
    }

    #[test]
    fn show_all_keeps_every_row_and_cuts_nothing() {
        let mut items: Vec<u32> = (0..45).collect();
        assert!(!cut_group(&mut items, 20, true));
        assert_eq!(items.len(), 45);
    }

    #[test]
    fn deadlines_read_as_a_person_says_them() {
        let today = "2026-11-04".parse().expect("date");
        assert_eq!(due_label("2026-11-04", today), "Due today");
        assert_eq!(due_label("2026-11-05", today), "Due tomorrow");
        assert_eq!(due_label("2026-11-15", today), "Due 15 Nov");
        assert_eq!(due_label("2026-11-15T17:30", today), "Due 15 Nov, 17:30");
        assert_eq!(due_label("2026-11-05T09:00", today), "Due tomorrow, 09:00");
        assert_eq!(due_label("not a date", today), "Has a deadline");
    }

    #[test]
    fn displayed_text_has_no_carriage_returns_for_the_input_to_draw() {
        assert_eq!(display_text("one\r\ntwo\rthree\n"), "one\ntwo\nthree\n");
        assert_eq!(display_text("Bonjour · 世界"), "Bonjour · 世界");
    }

    #[test]
    fn received_dates_use_declared_zone_across_the_autumn_clock_change() {
        // Two UTC instants map to the repeated Paris 02:30, with distinct offsets.
        let before = chrono::DateTime::parse_from_rfc3339("2026-10-25T00:30:00Z")
            .expect("instant")
            .timestamp();
        let after = before + 3600;
        assert_eq!(date(before, "Europe/Paris"), "25 Oct 2026 · 02:30 CEST");
        assert_eq!(date(after, "Europe/Paris"), "25 Oct 2026 · 02:30 CET");
        assert_eq!(date(after, "UTC"), "25 Oct 2026 · 01:30 UTC");
        assert_eq!(date(i64::MAX, "UTC"), "Date unavailable");
    }
}
