use std::fmt;

/// Скидка тенанта в процентах.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct DiscountRate(u8);

impl DiscountRate {
    pub const NONE: Self = Self(0);
    /// Скидка тенанта, у которого открытых тикетов больше [`Self::THRESHOLD`].
    pub const STANDARD: Self = Self(10);
    /// Сколько открытых тикетов нужно превысить, чтобы получить скидку.
    pub const THRESHOLD: usize = 5;

    /// Политика скидок: один порог.
    pub fn for_open_tickets(open_tickets: usize) -> Self {
        if open_tickets > Self::THRESHOLD {
            Self::STANDARD
        } else {
            Self::NONE
        }
    }

    pub fn percent(self) -> u8 {
        self.0
    }
}

impl fmt::Display for DiscountRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}%", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_discount_up_to_threshold() {
        for open_tickets in [0, 1, 5] {
            assert_eq!(
                DiscountRate::for_open_tickets(open_tickets),
                DiscountRate::NONE
            );
        }
    }

    #[test]
    fn discount_applies_above_threshold() {
        for open_tickets in [6, 10, 1000] {
            assert_eq!(
                DiscountRate::for_open_tickets(open_tickets),
                DiscountRate::STANDARD
            );
        }
        assert_eq!(DiscountRate::STANDARD.percent(), 10);
    }
}
