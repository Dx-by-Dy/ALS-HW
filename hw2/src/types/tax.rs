use rust_decimal::Decimal;

use crate::types::money::Money;

/// Налог заданый произвольной функцией от суммы, что
/// позволяет выражать правила, зависящие от самой цены (пороги,
/// ступенчатые шкалы), не расширяя тип.
#[repr(transparent)]
pub struct Tax(Box<dyn Fn(Money) -> Money + Send + Sync>);

impl Tax {
    /// Умножает сумму на `factor`: 1.2 -> плюс 20%.
    pub fn multiplier(factor: Decimal) -> Tax {
        Tax(Box::new(move |base| base * factor))
    }

    /// Налог в процентах от суммы: 20 -> плюс 20%.
    pub fn percent(percent: Decimal) -> Tax {
        Self::multiplier(Decimal::ONE + percent / Decimal::ONE_HUNDRED)
    }

    pub fn custom(rule: impl Fn(Money) -> Money + Send + Sync + 'static) -> Tax {
        Tax(Box::new(rule))
    }

    pub fn apply(&self, base: Money) -> Money {
        (self.0)(base)
    }

    pub fn identical() -> Tax {
        Tax(Box::new(move |base| base))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn money(mantissa: i64, scale: u32) -> Money {
        Money::from(Decimal::new(mantissa, scale))
    }

    #[test]
    fn multiplier_scales_the_base() {
        assert_eq!(Tax::multiplier(Decimal::new(12, 1)).apply(money(10000, 2)), money(12000, 2));
    }

    #[test]
    fn percent_adds_the_share() {
        assert_eq!(Tax::percent(Decimal::from(20)).apply(money(10000, 2)), money(12000, 2));
    }

    #[test]
    fn zero_percent_keeps_the_base() {
        assert_eq!(Tax::percent(Decimal::ZERO).apply(money(1234, 2)), money(1234, 2));
    }

    #[test]
    fn custom_rule_can_depend_on_the_amount() {
        let tax = Tax::custom(|base| if base > Money::ONE { base * Decimal::TWO } else { base });
        assert_eq!(tax.apply(money(1000, 2)), money(2000, 2));
    }

    #[test]
    fn a_rule_can_be_applied_many_times() {
        let tax = Tax::percent(Decimal::from(20));
        assert_eq!(tax.apply(money(10000, 2)), money(12000, 2));
        assert_eq!(tax.apply(money(5000, 2)), money(6000, 2));
    }

    #[test]
    fn identical_keeps_base_untouched() {
        assert_eq!(Tax::identical().apply(money(1234, 2)), money(1234, 2));
    }

    #[test]
    fn applying_to_zero_yields_zero() {
        assert_eq!(Tax::percent(Decimal::from(20)).apply(Money::ZERO), Money::ZERO);
    }
}
