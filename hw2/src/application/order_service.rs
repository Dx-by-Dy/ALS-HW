use crate::domain::notifier::{NotifyError, OrderNotifier};
use crate::domain::order::{Order, OrderError, OrderEvent};

pub struct OrderService<N> {
    notifier: N,
}

impl<N: OrderNotifier> OrderService<N> {
    pub fn new(notifier: N) -> Self {
        Self { notifier }
    }

    pub fn pay(&self, order: &mut Order) -> Result<Option<NotifyError>, OrderError> {
        self.apply(order, Order::pay)
    }

    pub fn post(&self, order: &mut Order) -> Result<Option<NotifyError>, OrderError> {
        self.apply(order, Order::post)
    }

    fn apply<F>(&self, order: &mut Order, transition: F) -> Result<Option<NotifyError>, OrderError>
    where
        F: FnOnce(&mut Order) -> Result<OrderEvent, OrderError>,
    {
        let event = transition(order)?;
        Ok(self.notifier.notify(order, &event).err())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::notifier::test_doubles::SpyNotifier;
    use crate::domain::order::{ItemId, OrderId, OrderItem, OrderStatus};
    use crate::types::money::Money;

    fn order_with_items() -> Order {
        let mut order = Order::new(OrderId::from(1));
        order
            .add_item(OrderItem::new(ItemId::from(1), 1, Money::ONE).expect("valid item"))
            .expect("order is pending");
        order
    }

    #[test]
    fn pay_changes_status_and_notifies() {
        let service = OrderService::new(SpyNotifier::new());
        let mut order = order_with_items();

        let failure = service.pay(&mut order).expect("transition is allowed");

        assert!(failure.is_none());
        assert_eq!(order.status(), OrderStatus::Paid);
        assert_eq!(
            service.notifier.sent(),
            vec![OrderEvent::StatusChanged {
                id: OrderId::from(1),
                from: OrderStatus::Pending,
                to: OrderStatus::Paid,
            }]
        );
    }

    #[test]
    fn post_changes_status_and_notifies() {
        let service = OrderService::new(SpyNotifier::new());
        let mut order = order_with_items();
        service.pay(&mut order).unwrap();

        service.post(&mut order).expect("transition is allowed");

        assert_eq!(order.status(), OrderStatus::Posted);
        assert_eq!(service.notifier.sent().len(), 2);
    }

    #[test]
    fn rejected_transition_notifies_nobody() {
        let service = OrderService::new(SpyNotifier::new());
        let mut order = order_with_items();

        let error = service.post(&mut order).unwrap_err();

        assert_eq!(
            error,
            OrderError::InvalidTransition { from: OrderStatus::Pending, to: OrderStatus::Posted }
        );
        assert!(service.notifier.sent().is_empty());
    }

    #[test]
    fn empty_order_is_rejected_before_any_notification() {
        let service = OrderService::new(SpyNotifier::new());
        let mut order = Order::new(OrderId::from(1));

        assert_eq!(service.pay(&mut order).unwrap_err(), OrderError::EmptyOrder);
        assert!(service.notifier.sent().is_empty());
    }

    #[test]
    fn failed_delivery_does_not_roll_back_the_transition() {
        let service = OrderService::new(SpyNotifier::failing());
        let mut order = order_with_items();

        let failure = service.pay(&mut order).expect("transition still succeeds");

        assert!(failure.is_some());
        assert_eq!(order.status(), OrderStatus::Paid);
    }

    #[test]
    fn events_are_not_delivered_twice() {
        let service = OrderService::new(SpyNotifier::new());
        let mut order = order_with_items();

        service.pay(&mut order).unwrap();
        service.post(&mut order).unwrap();

        let sent = service.notifier.sent();
        assert_eq!(sent.len(), 2);
        assert_ne!(sent[0], sent[1]);
    }

    #[test]
    fn service_works_with_any_notifier_implementation() {
        struct SilentNotifier;
        impl OrderNotifier for SilentNotifier {
            fn notify(&self, _order: &Order, _event: &OrderEvent) -> Result<(), NotifyError> {
                Ok(())
            }
        }

        let service = OrderService::new(SilentNotifier);
        let mut order = order_with_items();
        assert!(service.pay(&mut order).unwrap().is_none());
        assert_eq!(order.status(), OrderStatus::Paid);
    }
}
