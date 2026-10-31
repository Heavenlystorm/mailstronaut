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
}

impl Row {
    fn same_group(&self, other: &Self) -> bool {
        self.account == other.account
            && match (self.conversation, other.conversation) {
                (Some(a), Some(b)) => a == b,
                _ => self.id == other.id,
            }
    }
}

#[derive(Default)]
pub struct Rows {
    pub items: Vec<Row>,
    pub loading: bool,
    pub searching: bool,
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
                    "{} · {} · {} · {} messages · {} unread",
                    row.sender, row.subject, row.account, row.messages, row.unread
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
                                .truncate(),
                        )
                        .child(
                            Label::new(if row.messages > 1 {
                                format!("{} · {} messages", row.account, row.messages)
                            } else {
                                row.account.clone()
                            })
                            .text_xs()
                            .text_color(muted)
                            .truncate(),
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
