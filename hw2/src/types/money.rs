use derive_more::{Add, AddAssign, From, Mul, MulAssign, Sub, SubAssign};
use rust_decimal::Decimal;
use std::fmt;

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Add,
    AddAssign,
    Mul,
    From,
    MulAssign,
    Sub,
    SubAssign,
)]
#[repr(transparent)]
pub struct Money(Decimal);

impl Money {
    pub const ZERO: Money = Money(Decimal::ZERO);
    pub const ONE: Money = Money(Decimal::ONE);

    pub fn is_negative(&self) -> bool {
        self.0 < Decimal::ZERO
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.2}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn money(mantissa: i64, scale: u32) -> Money {
        Money::from(Decimal::new(mantissa, scale))
    }

    #[test]
    fn default_is_zero() {
        assert_eq!(Money::default(), Money::ZERO);
    }

    #[test]
    fn constants_match_decimal_counterparts() {
        assert_eq!(Money::ZERO, Money::from(Decimal::ZERO));
        assert_eq!(Money::ONE, Money::from(Decimal::ONE));
    }

    #[test]
    fn scale_does_not_affect_equality() {
        assert_eq!(money(100, 2), Money::ONE);
    }

    #[test]
    fn add_and_sub_produce_new_values() {
        assert_eq!(money(1050, 2) + money(250, 2), money(1300, 2));
        assert_eq!(money(1300, 2) - money(250, 2), money(1050, 2));
    }

    #[test]
    fn add_assign_sums_values() {
        let mut sum = money(1050, 2);
        sum += money(250, 2);
        assert_eq!(sum, money(1300, 2));
    }

    #[test]
    fn sub_assign_subtracts_values() {
        let mut sum = money(1300, 2);
        sum -= money(250, 2);
        assert_eq!(sum, money(1050, 2));
    }

    #[test]
    fn mul_scales_by_decimal() {
        assert_eq!(money(1050, 2) * Decimal::from(3), money(3150, 2));
    }

    #[test]
    fn mul_by_zero_gives_zero() {
        assert_eq!(money(1050, 2) * Decimal::ZERO, Money::ZERO);
    }

    #[test]
    fn mul_assign_scales_in_place() {
        let mut price = money(1050, 2);
        price *= Decimal::from(2);
        assert_eq!(price, money(2100, 2));
    }

    #[test]
    fn ordering_follows_numeric_value() {
        assert!(money(999, 2) < money(1000, 2));
        assert!(money(1000, 2) >= Money::from(Decimal::from(10)));
    }

    #[test]
    fn is_negative_only_below_zero() {
        assert!(!Money::ZERO.is_negative());
        assert!(!Money::ONE.is_negative());
        assert!(Money::from(-Decimal::ONE).is_negative());
    }

    #[test]
    fn display_keeps_two_decimal_places() {
        assert_eq!(money(1050, 2).to_string(), "10.50");
        assert_eq!(Money::ONE.to_string(), "1.00");
    }
}
