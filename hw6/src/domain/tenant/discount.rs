use std::fmt;

/// Скидка тенанта в процентах.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct DiscountRate(u8);

impl DiscountRate {
    pub const NONE: Self = Self(0);

    /// Политика скидок
    pub fn for_open_tickets(open_tickets: usize) -> Self {
        match open_tickets {
            0..=2 => Self(0),
            3..=5 => Self(5),
            6..=9 => Self(10),
            _ => Self(15),
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
    fn rate_grows_with_open_tickets() {
        assert_eq!(DiscountRate::for_open_tickets(0), DiscountRate::NONE);
        assert_eq!(DiscountRate::for_open_tickets(2).percent(), 0);
        assert_eq!(DiscountRate::for_open_tickets(3).percent(), 5);
        assert_eq!(DiscountRate::for_open_tickets(6).percent(), 10);
        assert_eq!(DiscountRate::for_open_tickets(10).percent(), 15);
        assert_eq!(DiscountRate::for_open_tickets(1000).percent(), 15);
    }
}
