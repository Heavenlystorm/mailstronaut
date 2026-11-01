//! Standard GPUI Kit list; selection never authorizes reading a message.

use gpui::base::StyledExt as _;
use gpui::component::{
    ActiveTheme as _, IndexPath,
    badge::Badge,
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyTitle},
    label::Label,
    list::{ListDelegate, ListItem, ListState},
};
use gpui::{
    App, Context, IntoElement, ParentElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};

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
        }
    }
}

impl Row {
    pub const fn cursor(&self) -> mailstronaut_core::store::mailbox::Cursor {
        mailstronaut_core::store::mailbox::Cursor {
            received_at: self.received_at,
            message_id: self.id,
        }
    }
    pub fn same_group(&self, other: &Self) -> bool {
        self.account == other.account
            && match (self.conversation, other.conversation) {
                (Some(a), Some(b)) => a == b,
                _ => self.id == other.id,
            }
    }
}

/// Bounded keyset pages; refreshing flags never resets the current page.
#[derive(Default)]
pub struct Pages {
    pub after: Option<mailstronaut_core::store::mailbox::Cursor>,
    previous: Vec<Option<mailstronaut_core::store::mailbox::Cursor>>,
    pub more: bool,
}
impl Pages {
    pub fn next(&mut self, cursor: mailstronaut_core::store::mailbox::Cursor) {
        self.previous.push(self.after);
        self.after = Some(cursor);
    }
    pub fn previous(&mut self) {
        if let Some(after) = self.previous.pop() {
            self.after = after;
        }
    }
    pub const fn has_previous(&self) -> bool {
        !self.previous.is_empty()
    }
    pub const fn number(&self) -> usize {
        self.previous.len() + 1
    }
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

#[derive(Default)]
pub struct Rows {
    pub items: Vec<Row>,
    pub loading: bool,
    pub searching: bool,
    pub time_zone: String,
}

/// Keep passive selection by canonical group, without scrolling or confirming it.
pub fn replace(
    list: &mut ListState<Rows>,
    rows: Vec<Row>,
    window: &mut Window,
    cx: &mut Context<ListState<Rows>>,
) {
    let selected = list.selected_index().and_then(|index| {
        let old = list.delegate().items.get(index.row)?;
        rows.iter().position(|row| row.same_group(old))
    });
    list.delegate_mut().items = rows;
    list.delegate_mut().loading = false;
    list.set_selected_index(selected.map(IndexPath::new), window, cx);
    cx.notify();
}

impl ListDelegate for Rows {
    type Item = ListItem;

    fn items_count(&self, _: usize, _: &App) -> usize {
        self.items.len()
    }

    fn render_item(
        &mut self,
        index: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let row = self.items.get(index.row)?;
        let muted = cx.theme().muted_foreground;
        let unread = row.unread > 0;
        Some(
            ListItem::new(index)
                .h(px(124.))
                .px_4()
                .border_b_1()
                .border_color(cx.theme().border)
                .accessibility_label(format!(
                    "{} · {} · {} · {} messages · {} unread{}",
                    row.sender,
                    row.subject,
                    row.account,
                    row.messages,
                    row.unread,
                    if row.starred { " · starred" } else { "" }
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
                                .child(
                                    Badge::new()
                                        .count(row.unread as usize)
                                        .color(cx.theme().primary)
                                        .child(div().w_3().h_3()),
                                ),
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
                                .line_clamp(2),
                        )
                        .child(
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
                                    Label::new(date(row.received_at, &self.time_zone))
                                        .text_xs()
                                        .text_color(muted)
                                        .flex_none(),
                                ),
                        ),
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

#[cfg(test)]
mod tests {
    use super::{Pages, date, display_text};
    use mailstronaut_core::store::mailbox::Cursor;

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

    #[test]
    fn bounded_pages_can_return_to_the_first_page_without_changing_the_anchor() {
        let mut pages = Pages::default();
        let first = Cursor {
            received_at: 100,
            message_id: 200,
        };
        let second = Cursor {
            received_at: 50,
            message_id: 100,
        };
        pages.next(first);
        pages.next(second);
        assert_eq!(pages.number(), 3);
        pages.previous();
        assert_eq!(pages.after, Some(first));
        assert!(pages.has_previous());
        pages.previous();
        assert!(pages.after.is_none());
        assert!(!pages.has_previous());
        assert_eq!(pages.number(), 1);
    }
}
