//! Standard native snapshot dialog; exporting never writes the active source.

use std::{io::Write as _, sync::Arc};

use gpui::component::{
    Disableable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    clipboard::Clipboard,
    dialog::DialogFooter,
    input::{Textarea, TextareaState},
    notification::{Notification, NotificationDelivery},
};
use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div, px,
};
use tokio::runtime::Runtime;

pub struct Export {
    text: String,
    reader: Entity<TextareaState>,
    runtime: Arc<Runtime>,
    saving: bool,
}

impl Export {
    pub fn show(text: String, runtime: Arc<Runtime>, window: &mut Window, cx: &mut gpui::App) {
        let panel = cx.new(|cx| {
            let reader = cx.new(|cx| {
                let mut reader = TextareaState::new(window, cx);
                reader.set_value(text.clone(), window, cx);
                reader
            });
            Self {
                text,
                reader,
                runtime,
                saving: false,
            }
        });
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Export to Nix")
                .width(px(720.))
                .child(panel.clone())
        });
    }

    fn save(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.saving = true;
        cx.notify();
        let directory =
            directories::UserDirs::new().map_or_default(|dirs| dirs.home_dir().to_path_buf());
        let selected = cx.prompt_for_new_path(&directory, Some("mailstronaut-settings.nix"));
        let (runtime, text) = (self.runtime.clone(), self.text.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = match selected.await {
                Ok(Ok(Some(path))) => Some(
                    runtime
                        .spawn_blocking(move || write_new(&path, &text))
                        .await
                        .unwrap_or(Err("Export was interrupted.")),
                ),
                Ok(Ok(None)) => None,
                _ => Some(Err("The file chooser is unavailable. Nothing was saved.")),
            };
            _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                if let Some(result) = result {
                    let notification = match result {
                        Ok(()) => Notification::success("Native Nix snapshot saved."),
                        Err(message) => Notification::error(message),
                    };
                    window
                        .push_notification(notification.delivery(NotificationDelivery::InApp), cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

fn write_new(path: &std::path::Path, text: &str) -> Result<(), &'static str> {
    let parent = path
        .parent()
        .ok_or("The selected path has no parent directory.")?;
    let mut file =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| "Unable to create the export.")?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.as_file().sync_all())
        .map_err(|_| "Unable to write the export.")?;
    file.persist_noclobber(path).map_err(|error| {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            "Choose a new filename; the existing file was preserved."
        } else {
            "Unable to install the export."
        }
    })?;
    Ok(())
}

impl Render for Export {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().flex_col().gap_3()
            .child("Copy this snapshot into Home Manager settings, or save a native config file. It includes effective defaults and keyring references.")
            .child(Textarea::new(&self.reader).readonly(true).aria_label("Native Nix snapshot").accessibility_id("native-nix-snapshot").h(px(320.)).w_full())
            .child(DialogFooter::new()
                .child(Clipboard::new("copy-native-snapshot").value(self.text.clone()).accessibility_label("Copy native Nix snapshot").tooltip("Copy native Nix snapshot")
                    .on_copied(|_, window, cx| window.push_notification(Notification::success("Native Nix snapshot copied.").delivery(NotificationDelivery::InApp), cx)))
                .child(Button::new("save-native-snapshot").label(if self.saving { "Saving…" } else { "Save…" }).disabled(self.saving).on_click(cx.listener(|this, _, window, cx| this.save(window, cx))))
                .child(Button::new("close-native-snapshot").ghost().label("Close").on_click(|_, window, cx| window.close_dialog(cx))))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt as _, symlink},
    };
    #[test]
    fn snapshot_is_private_and_never_overwrites_a_file_or_link() {
        let root = tempfile::tempdir().expect("private export");
        let path = root.path().join("export.nix");
        super::write_new(&path, "{ schemaVersion = 1; }").expect("new export");
        assert_eq!(
            fs::metadata(&path).expect("metadata").permissions().mode() & 0o777,
            0o600
        );
        assert!(super::write_new(&path, "replace").is_err());
        let link = root.path().join("link.nix");
        symlink(&path, &link).expect("link");
        assert!(super::write_new(&link, "replace").is_err());
        assert!(link.is_symlink());
        assert_eq!(
            fs::read_to_string(&path).expect("preserved"),
            "{ schemaVersion = 1; }"
        );
    }
}
