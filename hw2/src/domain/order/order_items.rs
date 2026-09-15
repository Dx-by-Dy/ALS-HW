use derive_more::From;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::fmt;
use std::num::NonZeroU32;

use super::error::OrderError;
use crate::types::money::Money;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
#[repr(transparent)]
pub struct ItemId(u32);

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Default)]
pub struct OrderItems {
    items: HashMap<ItemId, OrderItem>,
    total_sum: Money,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderItem {
    id: ItemId,
    quantity: NonZeroU32,
    price: Money,
}

impl OrderItems {
    pub fn add_item(&mut self, item: OrderItem) -> Option<OrderItem> {
        self.total_sum += item.item_price();
        let replaced = self.items.insert(item.id, item);
        if let Some(old_item) = &replaced {
            self.total_sum -= old_item.item_price();
        }
        replaced
    }

    pub fn remove_item(&mut self, id: ItemId) -> Result<OrderItem, OrderError> {
        let removed = self.items.remove(&id).ok_or(OrderError::ItemNotFound(id))?;
        self.total_sum -= removed.item_price();
        Ok(removed)
    }

    pub fn total_sum(&self) -> Money {
        self.total_sum
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn get(&self, id: ItemId) -> Option<&OrderItem> {
        self.items.get(&id)
    }
}

impl OrderItem {
    pub fn new(id: ItemId, quantity: u32, price: Money) -> Result<Self, OrderError> {
        let quantity = NonZeroU32::new(quantity).ok_or(OrderError::ZeroQuantity)?;
        if price.is_negative() {
            return Err(OrderError::NegativePrice);
        }
        Ok(Self { id, quantity, price })
    }

    pub fn id(&self) -> ItemId {
        self.id
    }

    pub fn quantity(&self) -> u32 {
        self.quantity.get()
    }

    pub fn price(&self) -> Money {
        self.price
    }

    pub fn item_price(&self) -> Money {
        self.price * Decimal::from(self.quantity.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn money(mantissa: i64, scale: u32) -> Money {
        Money::from(Decimal::new(mantissa, scale))
    }

    fn item(id: u32, quantity: u32, price: Money) -> OrderItem {
        OrderItem::new(ItemId::from(id), quantity, price).expect("valid item")
    }

    #[test]
    fn new_rejects_zero_quantity() {
        assert_eq!(
            OrderItem::new(ItemId::from(1), 0, Money::ONE),
            Err(OrderError::ZeroQuantity)
        );
    }

    #[test]
    fn new_rejects_negative_price() {
        assert_eq!(
            OrderItem::new(ItemId::from(1), 1, Money::from(-Decimal::ONE)),
            Err(OrderError::NegativePrice)
        );
    }

    #[test]
    fn new_accepts_a_free_item() {
        assert!(OrderItem::new(ItemId::from(1), 1, Money::ZERO).is_ok());
    }

    #[test]
    fn item_price_is_price_times_quantity() {
        assert_eq!(item(1, 3, money(1050, 2)).item_price(), money(3150, 2));
    }

    #[test]
    fn empty_collection_has_zero_total() {
        let items = OrderItems::default();
        assert!(items.is_empty());
        assert_eq!(items.total_sum(), Money::ZERO);
    }

    #[test]
    fn add_item_accumulates_total() {
        let mut items = OrderItems::default();
        assert!(items.add_item(item(1, 2, money(1000, 2))).is_none());
        assert!(items.add_item(item(2, 1, money(550, 2))).is_none());
        assert_eq!(items.total_sum(), money(2550, 2));
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn adding_same_id_reports_the_replaced_item_and_adjusts_total() {
        let mut items = OrderItems::default();
        items.add_item(item(1, 2, money(1000, 2)));
        let replaced = items.add_item(item(1, 5, money(1000, 2)));
        assert_eq!(replaced.map(|old| old.quantity()), Some(2));
        assert_eq!(items.total_sum(), money(5000, 2));
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn remove_item_subtracts_stored_item_price() {
        let mut items = OrderItems::default();
        items.add_item(item(1, 2, money(1000, 2)));
        items.add_item(item(2, 1, money(550, 2)));
        let removed = items.remove_item(ItemId::from(1)).expect("item is present");
        assert_eq!(removed.item_price(), money(2000, 2));
        assert_eq!(items.total_sum(), money(550, 2));
    }

    #[test]
    fn removing_unknown_id_reports_an_error() {
        let mut items = OrderItems::default();
        items.add_item(item(1, 2, money(1000, 2)));
        assert_eq!(
            items.remove_item(ItemId::from(42)).unwrap_err(),
            OrderError::ItemNotFound(ItemId::from(42))
        );
        assert_eq!(items.total_sum(), money(2000, 2));
    }

    #[test]
    fn add_then_remove_everything_returns_to_zero() {
        let mut items = OrderItems::default();
        items.add_item(item(1, 2, money(1000, 2)));
        items.add_item(item(2, 3, money(700, 2)));
        items.remove_item(ItemId::from(1)).unwrap();
        items.remove_item(ItemId::from(2)).unwrap();
        assert_eq!(items.total_sum(), Money::ZERO);
        assert!(items.is_empty());
    }

    #[test]
    fn get_returns_the_stored_item() {
        let mut items = OrderItems::default();
        items.add_item(item(1, 4, money(250, 2)));
        assert_eq!(items.get(ItemId::from(1)).map(|i| i.quantity()), Some(4));
        assert!(items.get(ItemId::from(2)).is_none());
    }
}
