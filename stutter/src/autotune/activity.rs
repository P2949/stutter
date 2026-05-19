use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityLevel {
    #[default]
    Active,
    SlowingDown,
    Idle,
}

#[derive(Clone, Debug)]
pub struct ActivityClassifier {
    window: VecDeque<u64>,
    window_size: usize,
}

impl ActivityClassifier {
    pub fn new(window_size: usize) -> Self {
        Self {
            window: VecDeque::new(),
            window_size: window_size.max(1),
        }
    }

    pub fn push_interval(&mut self, scored_samples: u64) {
        self.window.push_back(scored_samples);
        while self.window.len() > self.window_size {
            self.window.pop_front();
        }
    }

    pub fn classify(&self) -> ActivityLevel {
        if self.window.len() < 3 {
            return ActivityLevel::Active;
        }

        let recent = self
            .window
            .iter()
            .rev()
            .take(3)
            .copied()
            .collect::<Vec<_>>();
        let last = recent[0];
        let middle = recent[1];
        let first = recent[2];

        if first == 0 && middle == 0 && last == 0 {
            return ActivityLevel::Idle;
        }

        if first > middle && middle > last && first > 0 && last.saturating_mul(2) < first {
            return ActivityLevel::SlowingDown;
        }

        ActivityLevel::Active
    }
}

impl Default for ActivityClassifier {
    fn default() -> Self {
        Self::new(5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fewer_than_three_samples_is_active() {
        let mut classifier = ActivityClassifier::new(5);
        classifier.push_interval(0);
        classifier.push_interval(0);

        assert_eq!(classifier.classify(), ActivityLevel::Active);
    }

    #[test]
    fn three_zero_intervals_is_idle() {
        let mut classifier = ActivityClassifier::new(5);
        classifier.push_interval(0);
        classifier.push_interval(0);
        classifier.push_interval(0);

        assert_eq!(classifier.classify(), ActivityLevel::Idle);
    }

    #[test]
    fn sharp_monotonic_decline_is_slowing_down() {
        let mut classifier = ActivityClassifier::new(5);
        classifier.push_interval(120);
        classifier.push_interval(60);
        classifier.push_interval(20);

        assert_eq!(classifier.classify(), ActivityLevel::SlowingDown);
    }
}
