//! Reuse CPU-rendered images between animation frames.
use std::{cell::Cell, time::{Duration, Instant}};

const FRAME_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Default)]
pub struct FrameClock {
    last: Cell<Option<Instant>>,
    size: Cell<(usize, usize, u32)>,
}

impl FrameClock {
    pub fn can_reuse(&self, width: usize, height: usize, scale: f32) -> bool {
        self.size.get() == (width, height, scale.to_bits())
            && self.last.get().is_some_and(|time| time.elapsed() < FRAME_INTERVAL)
    }

    pub fn rendered(&self, width: usize, height: usize, scale: f32) {
        self.size.set((width, height, scale.to_bits()));
        self.last.set(Some(Instant::now()));
    }

    pub fn invalidate(&self) {
        self.last.set(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_invalidates_for_size_scale_and_explicit_changes() {
        let clock = FrameClock::default();
        assert!(!clock.can_reuse(100, 200, 1.));
        clock.rendered(100, 200, 1.);
        assert!(clock.can_reuse(100, 200, 1.));
        assert!(!clock.can_reuse(101, 200, 1.));
        assert!(!clock.can_reuse(100, 200, 2.));
        clock.invalidate();
        assert!(!clock.can_reuse(100, 200, 1.));
    }
}
