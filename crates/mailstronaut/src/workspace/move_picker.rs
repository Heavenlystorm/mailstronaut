use std::collections::HashMap;

use super::{Scope, View, Workspace};
use gpui::component::{
    Disableable as _,
    button::{Button, ButtonVariants as _},
    combobox::{Combobox, ComboboxEvent, ComboboxState},
    searchable_list::{SearchableListItem, SearchableVec},
};
use gpui::{
    AnyElement, AppContext as _, Context, Entity, IntoElement as _, SharedString, Styled as _,
    Subscription, Window, px,
};
use mailstronaut_core::store::mailbox::Folder;

#[derive(Clone)]
struct Destination {
    name: String,
    title: SharedString,
}

impl SearchableListItem for Destination {
    type Value = String;
    fn title(&self) -> SharedString {
        self.title.clone()
    }
    fn value(&self) -> &String {
        &self.name
    }
}

fn destinations<'a>(folders: impl Iterator<Item = &'a Folder>, scope: &Scope) -> Vec<Destination> {
    let mut options: Vec<_> = folders
        .filter(|folder| match scope {
            Scope::Folder { folder: source, .. } => *source != folder.id,
            Scope::Inbox | Scope::Account(_) => !folder.inbox,
        })
        .map(|folder| Destination {
            name: folder.name.clone(),
            title: folder.label().into(),
        })
        .collect();
    let mut counts = HashMap::new();
    for item in &options {
        *counts.entry(item.title.clone()).or_insert(0usize) += 1;
    }
    for item in &mut options {
        if counts[&item.title] > 1 {
            item.title = format!("{} ({})", item.title, item.name).into();
        }
    }
    options
}

pub(super) struct Picker {
    state: Entity<ComboboxState<SearchableVec<Destination>>>,
    empty: bool,
    _events: Subscription,
}

impl Workspace {
    pub(super) fn reset_move_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = &self.selected else {
            self.move_picker = None;
            return;
        };
        let choices = destinations(self.account_folders(&row.account), &self.scope);
        let empty = choices.is_empty();
        let state = cx.new(|cx| {
            ComboboxState::new(SearchableVec::new(choices), Vec::new(), window, cx).searchable(true)
        });
        let generation = self.reading_generation;
        let revision = self.declaration.revision();
        let scope = self.scope.clone();
        let events = cx.subscribe_in(&state, window, move |this, picker, event, window, cx| {
            let ComboboxEvent::Confirm(values) = event else {
                return;
            };
            // With no initial selection, Escape and outside dismissal have no value.
            let [name] = values.as_slice() else { return };
            if this.view != View::Inbox
                || generation != this.reading_generation
                || revision != this.declaration.revision()
                || scope != this.scope
                || this
                    .move_picker
                    .as_ref()
                    .is_none_or(|active| &active.state != picker)
            {
                return;
            }
            let Some(selected) = &this.selected else {
                return;
            };
            let label = this
                .account_folders(&selected.account)
                .find(|folder| &folder.name == name)
                .map(Folder::label);
            if let Some(label) = label {
                this.move_open(name.clone(), label, window, cx);
            }
        });
        self.move_picker = Some(Picker {
            state,
            empty,
            _events: events,
        });
    }

    pub(super) fn move_picker_control(&self) -> AnyElement {
        let Some(picker) = &self.move_picker else {
            return Button::new("move-to")
                .ghost()
                .label("Move to")
                .disabled(true)
                .into_any_element();
        };
        Combobox::new(&picker.state)
            .placeholder("Move to")
            .search_placeholder("Search folders…")
            .menu_width(px(250.))
            .w(px(95.))
            .appearance(false)
            .cleanable(false)
            .disabled(self.demo || picker.empty)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailstronaut_core::store::mailbox::FolderRole;

    #[test]
    fn matching_display_labels_keep_distinct_server_destinations() {
        let folder = |id, name: &str, role| Folder {
            id,
            account: "a".into(),
            name: name.into(),
            uid_validity: None,
            inbox: id == 1,
            role,
        };
        let folders = [
            folder(1, "INBOX", None),
            folder(2, "Archive", None),
            folder(3, "All-Mail", Some(FolderRole::Archive)),
        ];
        let items = destinations(folders.iter(), &Scope::Inbox);
        assert_eq!(items.len(), 2);
        assert_ne!(items[0].title(), items[1].title());
        assert_eq!(items[0].value(), "Archive");
        assert_eq!(items[1].value(), "All-Mail");
        let from_archive = destinations(
            folders.iter(),
            &Scope::Folder {
                account: "a".into(),
                folder: 2,
            },
        );
        assert_eq!(from_archive[0].value(), "INBOX");
        assert_eq!(from_archive[1].value(), "All-Mail");
    }
}
