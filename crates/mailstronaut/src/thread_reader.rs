//! Bounded, metadata-only native conversation navigation. Confirmation opens one original.

use std::{collections::HashSet, rc::Rc, sync::Arc};

use gpui::component::{
    ActiveTheme as _, IconName, IndexPath,
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

use crate::mailbox::Row;

pub struct Open(pub Row);

type LoadMore = Rc<dyn Fn(&mut Window, &mut App)>;

async fn member_prefix(
    handle: &Handle,
    account: String,
    id: i64,
    boundary: Option<mailstronaut_core::store::mailbox::Cursor>,
) -> Result<(Vec<Summary>, bool), mailstronaut_core::worker::Error> {
    let mut rows = Vec::new();
    let mut after = None;
    loop {
        let batch = handle
            .conversation_member_page(account.clone(), id, after, 100)
            .await?;
        let next = batch.last().map(Summary::cursor);
        let full = batch.len() == 100;
        rows.extend(batch);
        let reached = boundary.is_none_or(|end| {
            next.is_none_or(|cursor| {
                (cursor.received_at, cursor.message_id) <= (end.received_at, end.message_id)
            })
        });
        if !full || reached {
            return Ok((rows, full));
        }
        after = next;
    }
}

#[derive(Default)]
struct Members {
    rows: Vec<Summary>,
    zone: String,
    loading: bool,
    more: bool,
    on_more: Option<LoadMore>,
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
    fn has_more(&self, _: &App) -> bool {
        self.more && !self.loading
    }
    fn load_more(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        if self.more
            && !self.loading
            && let Some(load) = self.on_more.clone()
        {
            self.more = false;
            window.defer(cx, move |window, cx| load(window, cx));
        }
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
    after: Option<mailstronaut_core::store::mailbox::Cursor>,
    pending: bool,
    work: Option<tokio::task::AbortHandle>,
    generation: u64,
    expanded: bool,
    notice: String,
}
impl EventEmitter<Open> for ThreadReader {}

impl Drop for ThreadReader {
    fn drop(&mut self) {
        if let Some(work) = self.work.take() {
            work.abort();
        }
    }
}

impl ThreadReader {
    pub fn new(runtime: Arc<Runtime>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let reader = cx.entity().downgrade();
        let members = Members {
            on_more: Some(Rc::new(move |window, cx| {
                _ = reader.update(cx, |this, cx| this.load_more(window, cx));
            })),
            ..Members::default()
        };
        let list = cx.new(|cx| ListState::new(members, window, cx).searchable(false));
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
            after: None,
            pending: false,
            work: None,
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
        if let Some(work) = self.work.take() {
            work.abort();
        }
        self.generation = self.generation.wrapping_add(1);
        self.handle = None;
        self.group = None;
        self.after = None;
        self.pending = false;
        self.expanded = false;
        self.notice.clear();
        self.list.update(cx, |list, cx| {
            list.delegate_mut().rows.clear();
            list.delegate_mut().loading = false;
            list.delegate_mut().more = false;
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
        if let Some(work) = self.work.take() {
            work.abort();
        }
        self.generation = self.generation.wrapping_add(1);
        self.pending = false;
        let generation = self.generation;
        let (account, id, boundary) = (group.account.clone(), group.id, self.after);
        self.list.update(cx, |list, cx| {
            list.delegate_mut().loading = true;
            cx.notify();
        });
        let job = self
            .runtime
            .spawn(async move { member_prefix(&handle, account, id, boundary).await });
        self.work = Some(job.abort_handle());
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if generation != this.generation || !this.expanded {
                    return;
                }
                this.work = None;
                this.list.update(cx, |list, cx| {
                    list.delegate_mut().loading = false;
                    match result {
                        Ok(Ok((rows, more))) => {
                            let selected = list
                                .selected_index()
                                .and_then(|index| list.delegate().rows.get(index.row))
                                .map(|row| row.id)
                                .or_else(|| this.group.as_ref().map(|group| group.id));
                            let old_offset = list.scroll_handle().offset();
                            let anchor = list
                                .delegate()
                                .rows
                                .iter()
                                .scan(px(0.), |y, row| {
                                    let here = *y;
                                    *y += px(64.);
                                    Some((row.id, here))
                                })
                                .find(|(_, y)| *y + px(64.) > -old_offset.y)
                                .map(|(id, y)| (id, -old_offset.y - y));
                            let selected_index =
                                rows.iter().position(|row| Some(row.id) == selected);
                            this.after = rows.last().map(Summary::cursor);
                            list.delegate_mut().rows = rows;
                            list.delegate_mut().more = more;
                            list.set_selected_index(selected_index.map(IndexPath::new), window, cx);
                            if let Some((id, delta)) = anchor
                                && old_offset.y < px(0.)
                                && let Some(y) = list
                                    .delegate()
                                    .rows
                                    .iter()
                                    .scan(px(0.), |y, row| {
                                        let here = *y;
                                        *y += px(64.);
                                        Some((row.id, here))
                                    })
                                    .find_map(|(row, y)| (row == id).then_some(y))
                            {
                                let mut offset = old_offset;
                                offset.y = -y - delta;
                                list.scroll_handle().set_offset(offset);
                            }
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

    fn load_more(&mut self, window: &Window, cx: &Context<Self>) {
        if !self.expanded || self.pending || self.list.read(cx).delegate().loading {
            return;
        }
        let (Some(handle), Some(group), Some(after)) =
            (self.handle.clone(), &self.group, self.after)
        else {
            return;
        };
        self.pending = true;
        let generation = self.generation;
        let (account, id) = (group.account.clone(), group.id);
        let job = self.runtime.spawn(async move {
            handle
                .conversation_member_page(account, id, Some(after), 100)
                .await
        });
        self.work = Some(job.abort_handle());
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, _, cx| {
                if generation != this.generation || !this.expanded {
                    return;
                }
                this.work = None;
                this.pending = false;
                this.list.update(cx, |list, cx| {
                    match result {
                        Ok(Ok(rows)) => {
                            let more = rows.len() == 100;
                            if let Some(last) = rows.last() {
                                this.after = Some(last.cursor());
                            }
                            let mut ids: HashSet<_> =
                                list.delegate().rows.iter().map(|row| row.id).collect();
                            list.delegate_mut()
                                .rows
                                .extend(rows.into_iter().filter(|row| ids.insert(row.id)));
                            list.delegate_mut().more = more;
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
}

impl Render for ThreadReader {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.group.as_ref().map_or(0, |group| group.messages);
        if count < 2 {
            return div();
        }
        let content = div()
            .child(
                div()
                    .h(px(if count == 2 { 132. } else { 196. }))
                    .child(List::new(&self.list).size_full()),
            )
            .when(!self.notice.is_empty(), |view| {
                view.child(Label::new(self.notice.clone()).text_xs()).child(
                    Button::new("retry-thread-metadata")
                        .ghost()
                        .label("Retry")
                        .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
                )
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
                            this.pending = false;
                            if let Some(work) = this.work.take() {
                                work.abort();
                            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use mailstronaut_core::{
        store::mailbox::{Flags, Membership},
        threads::Groups,
        worker::Worker,
    };
    use std::num::NonZeroU32;

    #[tokio::test]
    async fn refreshed_thread_prefix_keeps_older_boundary_when_a_new_original_arrives() {
        let root = tempfile::tempdir().expect("private profile");
        let owner = Worker::open(root.path().to_owned()).await.expect("worker");
        let store = owner.handle();
        store.ensure_account("a".into()).await.expect("account");
        let folder = store
            .ensure_folder("a".into(), "INBOX".into(), true)
            .await
            .expect("folder")
            .id;
        let validity = NonZeroU32::new(42).expect("validity");
        store
            .select_folder("a".into(), folder, validity)
            .await
            .expect("select");
        let mut ids = Vec::new();
        for uid in 1..=205 {
            ids.push(store.cache_message("a".into(), Membership { folder, uid_validity: validity, uid: NonZeroU32::new(uid).expect("uid"), flags: Flags::default() }, i64::from(uid), None, format!("Message-ID: <{uid}@example.test>\r\nSubject: Synthetic\r\n\r\nLocal fixture.").into_bytes()).await.expect("cache"));
        }
        let snapshot = store
            .conversation_snapshot("a".into())
            .await
            .expect("snapshot")
            .expect("projection needed");
        store
            .commit_conversations(
                snapshot,
                Groups {
                    groups: vec![ids.clone()],
                },
            )
            .await
            .expect("project");
        let (first, more) = member_prefix(&store, "a".into(), ids[0], None)
            .await
            .expect("first batch");
        assert!(more);
        assert_eq!(first.len(), 100);
        let boundary = first.last().expect("boundary").cursor();
        ids.push(
            store
                .cache_message(
                    "a".into(),
                    Membership {
                        folder,
                        uid_validity: validity,
                        uid: NonZeroU32::new(206).expect("uid"),
                        flags: Flags::default(),
                    },
                    206,
                    None,
                    b"Message-ID: <206@example.test>\r\nSubject: Synthetic\r\n\r\nNew.".to_vec(),
                )
                .await
                .expect("arrival"),
        );
        let snapshot = store
            .conversation_snapshot("a".into())
            .await
            .expect("snapshot")
            .expect("projection needed");
        store
            .commit_conversations(
                snapshot,
                Groups {
                    groups: vec![ids.clone()],
                },
            )
            .await
            .expect("project");
        let (refreshed, more) = member_prefix(&store, "a".into(), ids[0], Some(boundary))
            .await
            .expect("refresh");
        assert!(more);
        assert_eq!(refreshed.len(), 200);
        assert!(refreshed.iter().any(|row| row.id == boundary.message_id));
        assert_eq!(refreshed.first().expect("newest").id, ids[205]);
        assert!(refreshed.iter().all(|row| !row.seen));
        owner.shutdown().await.expect("shutdown");
    }
}
