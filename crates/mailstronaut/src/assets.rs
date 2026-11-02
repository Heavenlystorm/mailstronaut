//! Kit's component icons plus only the additional Lucide icons used by mail actions.

use std::borrow::Cow;

use gpui::{AssetSource, SharedString};

gpui::assets::icon_assets!(
    MailIcons,
    [
        SquarePen, RefreshCw, SunMoon, Send, Paperclip, Users, Trash, FileText
    ]
);

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if let Some(icon) = MailIcons.load(path)? {
            return Ok(Some(icon));
        }
        gpui::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        let mut paths = gpui::assets::Assets.list(path)?;
        paths.extend(MailIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::assets::IconName;

    #[test]
    fn every_mail_action_icon_is_bundled_alongside_component_icons() {
        for icon in [
            IconName::SquarePen,
            IconName::RefreshCw,
            IconName::SunMoon,
            IconName::Send,
            IconName::Paperclip,
            IconName::Users,
            IconName::Trash,
            IconName::FileText,
            IconName::Settings,
        ] {
            let bytes = Assets
                .load(&icon.path())
                .expect("bundled asset")
                .expect("SVG");
            assert!(!bytes.is_empty());
        }
    }
}
