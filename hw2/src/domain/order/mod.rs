pub mod error;
pub mod events;
mod order_items;
pub mod order_status;

use derive_more::From;
use std::fmt;

pub use error::OrderError;
pub use events::OrderEvent;
pub use order_items::{ItemId, OrderItem, OrderItems};
pub use order_status::OrderStatus;

use crate::domain::pricing::PricingPolicy;
use crate::types::money::Money;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
#[repr(transparent)]
pub struct OrderId(u32);

impl fmt::Display for OrderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug)]
pub struct Order {
    id: OrderId,
    status: OrderStatus,
    items: OrderItems,
}

impl Order {
    pub fn new(id: OrderId) -> Order {
        Self {
            id,
            status: Default::default(),
            items: Default::default(),
        }
    }
}

impl Order {
    pub fn add_item(&mut self, item: OrderItem) -> Result<Option<OrderItem>, OrderError> {
        self.ensure_items_mutable()?;
        Ok(self.items.add_item(item))
    }

    pub fn remove_item(&mut self, id: ItemId) -> Result<OrderItem, OrderError> {
        self.ensure_items_mutable()?;
        self.items.remove_item(id)
    }

    pub fn pay(&mut self) -> Result<OrderEvent, OrderError> {
        self.ensure_transition(OrderStatus::Paid)?;
        if self.items.is_empty() {
            return Err(OrderError::EmptyOrder);
        }
        Ok(self.commit_transition(OrderStatus::Paid))
    }

    pub fn post(&mut self) -> Result<OrderEvent, OrderError> {
        self.ensure_transition(OrderStatus::Posted)?;
        Ok(self.commit_transition(OrderStatus::Posted))
    }

    /// Сумма позиций без каких-либо правил.
    pub fn total_price(&self) -> Money {
        self.items.total_sum()
    }

    /// Сумма позиций с применением правил ценообразования
    pub fn total_with(&self, pricing: &PricingPolicy) -> Money {
        pricing.apply(self.total_price())
    }

    pub fn id(&self) -> OrderId {
        self.id
    }

    pub fn status(&self) -> OrderStatus {
        self.status
    }

    pub fn items(&self) -> &OrderItems {
        &self.items
    }

    fn ensure_items_mutable(&self) -> Result<(), OrderError> {
        if self.status.allows_item_changes() {
            Ok(())
        } else {
            Err(OrderError::ItemsLocked(self.status))
        }
    }

    fn ensure_transition(&self, next: OrderStatus) -> Result<(), OrderError> {
        if self.status.can_transition_to(next) {
            Ok(())
        } else {
            Err(OrderError::InvalidTransition { from: self.status, to: next })
        }
    }

    fn commit_transition(&mut self, next: OrderStatus) -> OrderEvent {
        let from = std::mem::replace(&mut self.status, next);
        OrderEvent::StatusChanged { id: self.id, from, to: next }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn money(mantissa: i64, scale: u32) -> Money {
        Money::from(Decimal::new(mantissa, scale))
    }

    fn item(id: u32, quantity: u32, price: Money) -> OrderItem {
        OrderItem::new(ItemId::from(id), quantity, price).expect("valid item")
    }

    fn pending_order_with_items() -> Order {
        let mut order = Order::new(OrderId::from(1));
        order.add_item(item(1, 2, money(1000, 2))).unwrap();
        order
    }

    #[test]
    fn new_order_is_empty_and_pending() {
        let order = Order::new(OrderId::from(7));
        assert_eq!(order.id(), OrderId::from(7));
        assert_eq!(order.status(), OrderStatus::Pending);
        assert_eq!(order.total_price(), Money::ZERO);
    }

    #[test]
    fn total_price_sums_all_items() {
        let mut order = Order::new(OrderId::from(1));
        order.add_item(item(1, 2, money(1000, 2))).unwrap();
        order.add_item(item(2, 1, money(550, 2))).unwrap();
        assert_eq!(order.total_price(), money(2550, 2));
    }

    #[test]
    fn remove_item_updates_total_price() {
        let mut order = pending_order_with_items();
        order.remove_item(ItemId::from(1)).unwrap();
        assert_eq!(order.total_price(), Money::ZERO);
    }

    #[test]
    fn pay_moves_pending_order_to_paid() {
        let mut order = pending_order_with_items();
        assert!(order.pay().is_ok());
        assert_eq!(order.status(), OrderStatus::Paid);
    }

    #[test]
    fn empty_order_cannot_be_paid() {
        let mut order = Order::new(OrderId::from(1));
        assert_eq!(order.pay().unwrap_err(), OrderError::EmptyOrder);
        assert_eq!(order.status(), OrderStatus::Pending);
    }

    #[test]
    fn order_cannot_be_paid_twice() {
        let mut order = pending_order_with_items();
        let _ = order.pay().unwrap();
        assert_eq!(
            order.pay().unwrap_err(),
            OrderError::InvalidTransition { from: OrderStatus::Paid, to: OrderStatus::Paid }
        );
    }

    #[test]
    fn unpaid_order_cannot_be_posted() {
        let mut order = pending_order_with_items();
        assert_eq!(
            order.post().unwrap_err(),
            OrderError::InvalidTransition { from: OrderStatus::Pending, to: OrderStatus::Posted }
        );
        assert_eq!(order.status(), OrderStatus::Pending);
    }

    #[test]
    fn happy_path_is_pending_paid_posted() {
        let mut order = pending_order_with_items();
        let _ = order.pay().unwrap();
        let _ = order.post().unwrap();
        assert_eq!(order.status(), OrderStatus::Posted);
    }

    #[test]
    fn items_are_frozen_after_payment() {
        let mut order = pending_order_with_items();
        let _ = order.pay().unwrap();
        assert_eq!(
            order.add_item(item(2, 1, Money::ONE)).unwrap_err(),
            OrderError::ItemsLocked(OrderStatus::Paid)
        );
        assert_eq!(
            order.remove_item(ItemId::from(1)).unwrap_err(),
            OrderError::ItemsLocked(OrderStatus::Paid)
        );
        assert_eq!(order.total_price(), money(2000, 2));
    }

    #[test]
    fn pay_returns_its_own_event() {
        let mut order = pending_order_with_items();
        assert_eq!(
            order.pay().unwrap(),
            OrderEvent::StatusChanged {
                id: OrderId::from(1),
                from: OrderStatus::Pending,
                to: OrderStatus::Paid,
            }
        );
    }

    #[test]
    fn post_returns_its_own_event() {
        let mut order = pending_order_with_items();
        let _ = order.pay().unwrap();
        assert_eq!(
            order.post().unwrap(),
            OrderEvent::StatusChanged {
                id: OrderId::from(1),
                from: OrderStatus::Paid,
                to: OrderStatus::Posted,
            }
        );
    }

    #[test]
    fn rejected_transition_produces_no_event() {
        let mut order = Order::new(OrderId::from(1));
        assert!(order.pay().is_err());
    }

    #[test]
    fn every_transition_reports_the_status_it_came_from() {
        let mut order = pending_order_with_items();
        let paid = order.pay().unwrap();
        let posted = order.post().unwrap();

        let OrderEvent::StatusChanged { from: first_from, .. } = paid;
        let OrderEvent::StatusChanged { from: second_from, to, .. } = posted;
        assert_eq!(first_from, OrderStatus::Pending);
        assert_eq!(second_from, OrderStatus::Paid);
        assert_eq!(to, OrderStatus::Posted);
    }

    #[test]
    fn changing_items_does_not_change_id_or_status() {
        let mut order = Order::new(OrderId::from(42));
        order.add_item(item(1, 1, Money::ONE)).unwrap();
        assert_eq!(order.id(), OrderId::from(42));
        assert_eq!(order.status(), OrderStatus::Pending);
    }
}
