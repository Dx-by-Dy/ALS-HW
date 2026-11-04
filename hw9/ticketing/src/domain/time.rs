use std::fmt;

/// Момент времени в секундах.
///
/// Время приходит в агрегат только через команды: `decide` остаётся чистой функцией,
/// а события фиксируют, когда именно всё произошло.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(pub u64);

impl Timestamp {
    /// Сколько секунд прошло от `earlier` до `self`; ноль, если `earlier` позже.
    pub fn secs_since(self, earlier: Timestamp) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "t={}s", self.0)
    }
}
