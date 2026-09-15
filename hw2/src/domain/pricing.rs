use crate::types::{discount::Discount, money::Money, tax::Tax};

/// Очерёдность применения правил.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplicationOrder {
    DiscountFirst,
    TaxFirst,
}

pub struct PricingPolicy {
    discount: Discount,
    tax: Tax,
    order: ApplicationOrder,
}

impl PricingPolicy {
    /// Скидка считается от базовой суммы, налог — от уже уменьшенной.
    pub fn discount_then_tax(discount: Discount, tax: Tax) -> Self {
        Self { discount, tax, order: ApplicationOrder::DiscountFirst }
    }

    /// Налог считается от базовой суммы, скидка — от суммы с налогом.
    pub fn tax_then_discount(discount: Discount, tax: Tax) -> Self {
        Self { discount, tax, order: ApplicationOrder::TaxFirst }
    }

    pub fn identical() -> Self {
        Self::discount_then_tax(Discount::identical(), Tax::identical())
    }

    pub fn apply(&self, base: Money) -> Money {
        match self.order {
            ApplicationOrder::DiscountFirst => self.tax.apply(self.discount.apply(base)),
            ApplicationOrder::TaxFirst => self.discount.apply(self.tax.apply(base)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::order::{ItemId, Order, OrderId, OrderItem};
    use rust_decimal::Decimal;

    fn money(mantissa: i64, scale: u32) -> Money {
        Money::from(Decimal::new(mantissa, scale))
    }

    fn order_for(total: Money) -> Order {
        let mut order = Order::new(OrderId::from(1));
        order
            .add_item(OrderItem::new(ItemId::from(1), 1, total).expect("valid item"))
            .expect("order is pending");
        order
    }

    fn ten_off() -> Discount {
        Discount::percent_off(Decimal::from(10))
    }

    fn vat() -> Tax {
        Tax::percent(Decimal::from(20))
    }

    #[test]
    fn discount_then_tax_taxes_the_reduced_amount() {
        let pricing = PricingPolicy::discount_then_tax(ten_off(), vat());
        assert_eq!(pricing.apply(money(10000, 2)), money(10800, 2));
    }

    #[test]
    fn tax_then_discount_discounts_the_taxed_amount() {
        let pricing = PricingPolicy::tax_then_discount(ten_off(), vat());
        assert_eq!(pricing.apply(money(10000, 2)), money(10800, 2));
    }

    #[test]
    fn multiplicative_rules_are_order_independent() {
        let base = money(133, 2);
        let (percent_off, percent) = (Decimal::from(15), Decimal::from(7));
        assert_eq!(
            PricingPolicy::discount_then_tax(
                Discount::percent_off(percent_off),
                Tax::percent(percent)
            )
            .apply(base),
            PricingPolicy::tax_then_discount(
                Discount::percent_off(percent_off),
                Tax::percent(percent)
            )
            .apply(base),
        );
    }

    #[test]
    fn threshold_rules_are_order_dependent() {
        let base = money(9500, 2);
        let threshold = money(10000, 2);
        let threshold_discount = || Discount::percent_off_above(Decimal::from(10), threshold);

        let discount_first =
            PricingPolicy::discount_then_tax(threshold_discount(), vat()).apply(base);
        let tax_first = PricingPolicy::tax_then_discount(threshold_discount(), vat()).apply(base);

        assert_eq!(discount_first, money(11400, 2), "скидка не сработала: 95.00 < порога");
        assert_eq!(tax_first, money(10260, 2), "налог поднял сумму до 114.00 — скидка сработала");
        assert_ne!(discount_first, tax_first);
    }

    #[test]
    fn identical_policy_keeps_the_amount() {
        assert_eq!(PricingPolicy::identical().apply(money(10000, 2)), money(10000, 2));
    }

    #[test]
    fn zero_stays_zero() {
        assert_eq!(PricingPolicy::discount_then_tax(ten_off(), vat()).apply(Money::ZERO), Money::ZERO);
    }

    #[test]
    fn a_policy_is_reusable_across_amounts() {
        // Ради этого правила и переведены на `Fn`: одна политика - много сумм.
        let pricing = PricingPolicy::discount_then_tax(ten_off(), vat());
        assert_eq!(pricing.apply(money(10000, 2)), money(10800, 2));
        assert_eq!(pricing.apply(money(5000, 2)), money(5400, 2));
        assert_eq!(pricing.apply(money(10000, 2)), money(10800, 2));
    }

    #[test]
    fn order_prices_itself_with_a_given_policy() {
        let order = order_for(money(10000, 2));
        let pricing = PricingPolicy::discount_then_tax(ten_off(), vat());

        assert_eq!(order.total_with(&pricing), money(10800, 2));
        assert_eq!(order.total_price(), money(10000, 2), "базовая сумма не тронута");
    }

    #[test]
    fn one_order_can_be_priced_by_several_policies() {
        let order = order_for(money(10000, 2));

        assert_eq!(order.total_with(&PricingPolicy::identical()), money(10000, 2));
        assert_eq!(
            order.total_with(&PricingPolicy::discount_then_tax(ten_off(), vat())),
            money(10800, 2)
        );
    }

    #[test]
    fn empty_order_stays_at_zero() {
        let order = Order::new(OrderId::from(1));
        let pricing = PricingPolicy::discount_then_tax(ten_off(), vat());
        assert_eq!(order.total_with(&pricing), Money::ZERO);
    }
}
