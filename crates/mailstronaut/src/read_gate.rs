//! Seen authorization comes only from an explicit message-open action.

#[derive(Default)]
pub struct Gate {
    visible: bool,
    clicked: Option<(u64, u64)>,
}
impl Gate {
    pub const fn visibility(&mut self, visible: bool) {
        self.visible = visible;
        if !visible {
            self.clicked = None;
        }
    }
    pub const fn clicked(&mut self, generation: u64, reading: u64) {
        self.clicked = Some((generation, reading));
    }
    pub fn take(&mut self, generation: u64, reading: u64, eligible: bool) -> bool {
        if self.visible && eligible && self.clicked == Some((generation, reading)) {
            self.clicked = None;
            true
        } else {
            false
        }
    }
    pub const fn suppress(&mut self, _generation: u64, _reading: u64) {
        self.clicked = None;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_an_explicit_open_authorizes_seen_once_its_body_is_ready() {
        let mut gate = Gate::default();
        gate.visibility(true);
        assert!(
            !gate.take(1, 1, true),
            "visibility, previews and restored selection never authorize Seen"
        );
        gate.clicked(1, 1);
        assert!(!gate.take(1, 1, false), "body must actually be available");
        assert!(gate.take(1, 1, true));
        assert!(!gate.take(1, 1, true), "refresh cannot repeat an open");
        gate.clicked(1, 2);
        assert!(
            !gate.take(1, 1, true),
            "old body cannot consume a newer click"
        );
        assert!(gate.take(1, 2, true));
        gate.clicked(1, 3);
        assert!(
            !gate.take(2, 3, true),
            "retired configuration cannot authorize another account"
        );
    }
    #[test]
    fn focus_or_page_return_and_explicit_unread_require_another_click() {
        let mut gate = Gate::default();
        gate.visibility(true);
        gate.clicked(1, 1);
        gate.visibility(false);
        gate.visibility(true);
        assert!(!gate.take(1, 1, true));
        gate.clicked(1, 2);
        gate.suppress(1, 2);
        assert!(!gate.take(1, 2, true));
        gate.visibility(false);
        gate.visibility(true);
        assert!(!gate.take(1, 2, true));
        gate.clicked(1, 3);
        assert!(
            gate.take(1, 3, true),
            "another explicit open can mark unread mail seen"
        );
    }
}
