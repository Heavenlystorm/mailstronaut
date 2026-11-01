//! Bounded, metadata-only native conversation navigation. Confirmation opens one original.

use std::sync::Arc;

use gpui::component::{
    ActiveTheme as _, Disableable as _, IconName, IndexPath,
    button::{Button, ButtonVariants as _},
    collapsible::Collapsible,
    label::Label,
    list::{List, ListDelegate, ListEvent, ListItem, ListState},
};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};
use mailstronaut_core::{store::mailbox::Summary, worker::Handle};
use tokio::runtime::Runtime;

use crate::mailbox::{Pages, Row};

pub struct Open(pub Row);

#[derive(Default)]
struct Members {
    rows: Vec<Summary>,
    zone: String,
    loading: bool,
}

impl ListDelegate for Members {
    type Item = ListItem;
    fn items_count(&self, _: usize, _: &App) -> usize {
        self.rows.len()
    }
    fn render_item(
        &mut self,
        index: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let row = self.rows.get(index.row)?;
        Some(
            ListItem::new(index)
                .h(px(64.))
                .px_2()
                .accessibility_label(format!(
                    "{} · {} · {}",
                    row.sender,
                    row.subject,
                    if row.seen { "Read" } else { "Unread" }
                ))
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .items_center()
                                .child(Label::new(row.sender.clone()).flex_1().min_w_0().truncate())
                                .child(
                                    Label::new(if row.seen { "Read" } else { "Unread" })
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground),
                                ),
                        )
                        .child(
                            Label::new(format!(
                                "{} · {}",
                                row.subject,
                                crate::mailbox::date(row.received_at, &self.zone)
                            ))
                            .truncate()
                            .text_xs(),
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
        cx.notify();
    }
    fn loading(&self, _: &App) -> bool {
        self.loading && self.rows.is_empty()
    }
}

pub struct ThreadReader {
    runtime: Arc<Runtime>,
    handle: Option<Handle>,
    group: Option<Row>,
    list: Entity<ListState<Members>>,
    _events: Subscription,
    pages: Pages,
    generation: u64,
    expanded: bool,
    notice: String,
}
impl EventEmitter<Open> for ThreadReader {}

impl ThreadReader {
    pub fn new(runtime: Arc<Runtime>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let list = cx.new(|cx| ListState::new(Members::default(), window, cx).searchable(false));
        let events = cx.subscribe_in(&list, window, |this, list, event, _, cx| {
            if let ListEvent::Confirm(index) = event
                && this.expanded
                && let Some(member) = list.read(cx).delegate().rows.get(index.row)
                && let Some(group) = &this.group
            {
                cx.emit(Open(Row {
                    account: member.account.clone(),
                    id: member.id,
                    conversation: group.conversation,
                    sender: member.sender.clone(),
                    subject: member.subject.clone(),
                    preview: member.preview.clone(),
                    unread: u32::from(!member.seen),
                    messages: group.messages,
                    received_at: member.received_at,
                    seen: member.seen,
                    starred: member.starred,
                    due: None,
                    expires_at: None,
                }));
            }
        });
        Self {
            runtime,
            handle: None,
            group: None,
            list,
            _events: events,
            pages: Pages::default(),
            generation: 0,
            expanded: false,
            notice: String::new(),
        }
    }

    pub fn show(
        &mut self,
        group: Row,
        handle: Handle,
        zone: String,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .group
            .as_ref()
            .is_none_or(|old| !old.same_group(&group))
        {
            self.clear(cx);
        }
        self.group = Some(group);
        self.handle = Some(handle);
        self.list
            .update(cx, |list, _| list.delegate_mut().zone = zone);
        self.refresh(window, cx);
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.handle = None;
        self.group = None;
        self.pages = Pages::default();
        self.expanded = false;
        self.notice.clear();
        self.list.update(cx, |list, cx| {
            list.delegate_mut().rows.clear();
            list.delegate_mut().loading = false;
            cx.notify();
        });
        cx.notify();
    }

    pub fn refresh(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.expanded {
            return;
        }
        let (Some(handle), Some(group)) = (self.handle.clone(), &self.group) else {
            return;
        };
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let (account, id, after) = (group.account.clone(), group.id, self.pages.after);
        self.list.update(cx, |list, cx| {
            list.delegate_mut().loading = true;
            cx.notify();
        });
        let job = self.runtime.spawn(async move {
            handle
                .conversation_member_page(account, id, after, 100)
                .await
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if generation != this.generation || !this.expanded {
                    return;
                }
                this.list.update(cx, |list, cx| {
                    list.delegate_mut().loading = false;
                    match result {
                        Ok(Ok(rows)) => {
                            this.pages.more = rows.len() == 100;
                            let selected = list
                                .selected_index()
                                .and_then(|index| list.delegate().rows.get(index.row))
                                .map(|row| row.id)
                                .or_else(|| this.group.as_ref().map(|group| group.id));
                            let index = rows.iter().position(|row| Some(row.id) == selected);
                            list.delegate_mut().rows = rows;
                            list.set_selected_index(index.map(IndexPath::new), window, cx);
                            this.notice.clear();
                        }
                        Ok(Err(error)) => this.notice = error.to_string(),
                        Err(_) => this.notice = "Unable to load cached thread metadata.".into(),
                    }
                    cx.notify();
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn turn_page(&mut self, older: bool, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.list.read(cx).delegate();
        if rows.loading {
            return;
        }
        if older {
            let Some(cursor) = rows
                .rows
                .last()
                .filter(|_| self.pages.more)
                .map(Summary::cursor)
            else {
                return;
            };
            self.pages.next(cursor);
        } else if self.pages.has_previous() {
            self.pages.previous();
        } else {
            return;
        }
        self.list.update(cx, |list, cx| {
            list.delegate_mut().rows.clear();
            list.set_selected_index(None, window, cx);
            list.scroll_to_item(IndexPath::new(0), gpui::ScrollStrategy::Top, window, cx);
        });
        self.refresh(window, cx);
    }
}

impl Render for ThreadReader {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.group.as_ref().map_or(0, |group| group.messages);
        if count < 2 {
            return div();
        }
        let loading = self.list.read(cx).delegate().loading;
        let footer = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                Button::new("newer-originals")
                    .ghost()
                    .label("Newer")
                    .disabled(loading || !self.pages.has_previous())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.turn_page(false, window, cx);
                    })),
            )
            .child(Label::new(format!("Page {}", self.pages.number())).text_xs())
            .child(
                Button::new("older-originals")
                    .ghost()
                    .label("Older")
                    .disabled(loading || !self.pages.more)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.turn_page(true, window, cx);
                    })),
            );
        let content = div()
            .child(
                div()
                    .h(px(if count == 2 { 132. } else { 196. }))
                    .child(List::new(&self.list).size_full()),
            )
            .when(self.pages.more || self.pages.has_previous(), |view| {
                view.child(footer)
            })
            .when(!self.notice.is_empty(), |view| {
                view.child(Label::new(self.notice.clone()).text_xs())
            });
        div().mb_3().child(
            Collapsible::new()
                .open(self.expanded)
                .child(
                    Button::new("thread-originals")
                        .ghost()
                        .label(format!("{count} messages"))
                        .icon(if self.expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .tooltip("Choose an original · arrow keys select, Enter opens")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.expanded = !this.expanded;
                            this.generation = this.generation.wrapping_add(1);
                            if this.expanded {
                                this.refresh(window, cx);
                                this.list.update(cx, |list, cx| list.focus(window, cx));
                            }
                            cx.notify();
                        })),
                )
                .content(content),
        )
    }
}
