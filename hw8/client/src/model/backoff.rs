//! Пауза перед повторным подключением: растёт после каждой неудачи, сбрасывается после успеха.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    next: Duration,
}

impl Backoff {
    pub const FIRST: Duration = Duration::from_millis(500);
    pub const MAX: Duration = Duration::from_secs(10);

    /// Пауза после очередной неудачи; следующая будет вдвое длиннее, но не больше [`Self::MAX`].
    pub fn failed(&mut self) -> Duration {
        let delay = self.next;
        self.next = (delay * 2).min(Self::MAX);
        delay
    }

    pub fn succeeded(&mut self) {
        self.next = Self::FIRST;
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self { next: Self::FIRST }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_doubles_up_to_limit_and_resets_after_success() {
        let mut backoff = Backoff::default();
        let delays: Vec<_> = (0..7).map(|_| backoff.failed().as_millis()).collect();
        assert_eq!(delays, [500, 1000, 2000, 4000, 8000, 10_000, 10_000]);
        backoff.succeeded();
        assert_eq!(backoff.failed(), Backoff::FIRST);
    }
}
