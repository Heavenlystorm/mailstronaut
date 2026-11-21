//! Durable server-history coverage for the current mailbox and its search results.

use std::collections::BTreeSet;

use gpui::{Context, Task};
use mailstronaut_core::store::{
    backfill::{Coverage, FolderCoverage},
    browse::Scope,
};

use super::Workspace;

impl Workspace {
    pub(super) fn refresh_history(&mut self, cx: &Context<Self>) {
        let Some(store) = self.handle() else {
            self.history_task = Task::ready(());
            return;
        };
        let accounts = self.accounts.clone();
        self.history_task = cx.spawn(async move |this, cx| {
            let history = store.coverage(accounts).await.ok();
            _ = this.update(cx, |this, cx| {
                this.history = history;
                cx.notify();
            });
        });
    }
}

pub(super) fn label(
    scope: &Scope,
    accounts: &[String],
    history: &[FolderCoverage],
    limit: Option<u32>,
) -> Option<String> {
    if accounts.is_empty() {
        return None;
    }
    let mut covered = BTreeSet::new();
    let mut incomplete = false;
    for folder in history {
        let in_scope = match scope {
            Scope::Inbox => folder.inbox && accounts.contains(&folder.account),
            Scope::Account(account) => *account == folder.account,
            Scope::Folder {
                account,
                folder: id,
            } => *account == folder.account && *id == folder.id,
        };
        if !in_scope {
            continue;
        }
        covered.insert(&folder.account);
        incomplete |= folder.coverage != Coverage::Complete { cap_days: limit };
    }
    let expected = if *scope == Scope::Inbox {
        accounts.len()
    } else {
        1
    };
    if incomplete || covered.len() < expected {
        Some("Server history incomplete".into())
    } else {
        limit.map(|days| format!("History fetch limited to {days} days"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_and_capped_history_stay_scoped_to_the_mail_being_browsed() {
        let accounts = ["a".into(), "b".into()];
        let complete = FolderCoverage {
            id: 1,
            account: "a".into(),
            folder: "INBOX".into(),
            inbox: true,
            coverage: Coverage::Complete { cap_days: None },
        };
        let sent = FolderCoverage {
            id: 2,
            folder: "Sent".into(),
            inbox: false,
            coverage: Coverage::NotStarted,
            ..complete.clone()
        };
        let capped = FolderCoverage {
            id: 3,
            account: "b".into(),
            coverage: Coverage::Complete { cap_days: Some(30) },
            ..complete.clone()
        };
        let first = Scope::Folder {
            account: "a".into(),
            folder: 1,
        };
        let second = Scope::Folder {
            account: "b".into(),
            folder: 3,
        };
        let pending = Some("Server history incomplete".into());
        for history in [&[][..], &[complete.clone()][..]] {
            assert_eq!(label(&Scope::Inbox, &accounts, history, None), pending);
        }
        let history = [complete, sent, capped];
        assert_eq!(label(&first, &accounts, &history, None), None);
        assert_eq!(
            label(&Scope::Account("a".into()), &accounts, &history, None),
            pending
        );
        assert_eq!(label(&second, &accounts, &history, None), pending);
        assert_eq!(
            label(&second, &accounts, &history, Some(30)).as_deref(),
            Some("History fetch limited to 30 days")
        );
    }
}
