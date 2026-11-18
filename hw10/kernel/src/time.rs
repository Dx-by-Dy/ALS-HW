use std::fmt;

/// Секунд в сутках.
pub const DAY_SECS: u64 = 24 * 60 * 60;

/// Момент времени в секундах.
///
/// Время приходит в модель только снаружи: решения остаются чистыми функциями,
/// а события фиксируют, когда именно всё произошло.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(pub u64);

impl Timestamp {
    /// Сколько секунд прошло от `earlier` до `self`; ноль, если `earlier` позже.
    pub fn secs_since(self, earlier: Timestamp) -> u64 {
        self.0.saturating_sub(earlier.0)
    }

    /// Момент через `secs` секунд после `self`.
    pub fn plus_secs(self, secs: u64) -> Timestamp {
        Timestamp(self.0.saturating_add(secs))
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "t={}s", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_time_never_goes_negative() {
        assert_eq!(Timestamp(10).secs_since(Timestamp(4)), 6);
        assert_eq!(Timestamp(4).secs_since(Timestamp(10)), 0);
    }

    #[test]
    fn adding_time_saturates_instead_of_overflowing() {
        assert_eq!(Timestamp(5).plus_secs(DAY_SECS), Timestamp(5 + 86_400));
        assert_eq!(Timestamp(u64::MAX - 1).plus_secs(10), Timestamp(u64::MAX));
    }
}
