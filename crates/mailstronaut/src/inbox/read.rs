//! Visibility/selection fencing for a short native-reader dwell, without a daemon.

#[derive(Default)]
pub(super) struct Gate {
    visible: bool,
    epoch: u64,
    armed: Option<(u64, u64, u64)>,
    consumed: Option<(u64, u64)>,
}
impl Gate {
    pub const fn visibility(&mut self, visible: bool) {
        if self.visible != visible {
            self.visible = visible;
            self.epoch = self.epoch.saturating_add(1);
            self.armed = None;
        }
    }
    pub fn arm(
        &mut self,
        generation: u64,
        reading: u64,
        eligible: bool,
    ) -> Option<(u64, u64, u64)> {
        if !self.visible || !eligible || self.consumed == Some((generation, reading)) {
            return None;
        }
        let stamp = (generation, reading, self.epoch);
        if self.armed == Some(stamp) {
            return None;
        }
        self.armed = Some(stamp);
        Some(stamp)
    }
    pub const fn suppress(&mut self, generation: u64, reading: u64) {
        self.consumed = Some((generation, reading));
        self.armed = None;
    }
    pub fn consume(
        &mut self,
        stamp: (u64, u64, u64),
        generation: u64,
        reading: u64,
        eligible: bool,
    ) -> bool {
        if self.armed != Some(stamp) {
            return false;
        }
        self.armed = None;
        if !eligible
            || !self.visible
            || stamp != (generation, reading, self.epoch)
            || self.consumed == Some((generation, reading))
        {
            return false;
        }
        self.suppress(generation, reading);
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn focus_body_readiness_and_navigation_fence_every_timer() {
        let mut gate = Gate::default();
        assert_eq!(gate.arm(1, 1, true), None);
        gate.visibility(true);
        assert_eq!(gate.arm(1, 1, false), None);
        let old = gate.arm(1, 1, true).expect("first dwell");
        assert_eq!(gate.arm(1, 1, true), None);
        gate.visibility(false);
        assert!(!gate.consume(old, 1, 1, true));
        gate.visibility(true);
        let fresh = gate.arm(1, 1, true).expect("new focus dwell");
        assert_ne!(old, fresh);
        assert!(!gate.consume(old, 1, 1, true));
        assert!(gate.consume(fresh, 1, 1, true));
        assert_eq!(gate.arm(1, 1, true), None);
        let next = gate.arm(1, 2, true).expect("next message");
        assert!(!gate.consume(next, 1, 3, true));
        let retired = gate.arm(1, 3, true).expect("retired configuration");
        assert!(!gate.consume(retired, 2, 3, true));
        let not_ready = gate.arm(2, 3, true).expect("readiness may change");
        assert!(!gate.consume(not_ready, 2, 3, false));
        assert!(
            gate.arm(2, 3, true).is_some(),
            "a temporary edit in flight does not lose the opportunity forever"
        );
    }
    #[test]
    fn explicit_unread_or_undo_is_not_overridden_until_another_open() {
        let mut gate = Gate::default();
        gate.visibility(true);
        let timer = gate.arm(1, 1, true).expect("dwell");
        gate.suppress(1, 1);
        assert!(!gate.consume(timer, 1, 1, true));
        gate.visibility(false);
        gate.visibility(true);
        assert_eq!(gate.arm(1, 1, true), None);
        assert!(gate.arm(1, 2, true).is_some());
    }
}
