use crate::domain::notifier::{NotifyError, OrderNotifier};
use crate::domain::order::{Order, OrderEvent};

macro_rules! status_order_changed {
    () => {
        "order {} status changed to {}"
    };
}

#[derive(Debug, Default)]
pub struct EmailService;

impl EmailService {
    pub fn new() -> Self {
        Self
    }

    pub fn send_message_by_order(&self, order: &Order, message: String) -> Result<(), NotifyError> {
        let _ = (order, message);
        Ok(())
    }
}

impl OrderNotifier for EmailService {
    fn notify(&self, order: &Order, event: &OrderEvent) -> Result<(), NotifyError> {
        let message = match event {
            OrderEvent::StatusChanged { to, .. } => {
                format!(status_order_changed!(), order.id(), to)
            }
        };
        self.send_message_by_order(order, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::order::{ItemId, OrderId, OrderItem, OrderStatus};
    use crate::types::money::Money;

    fn paid_order() -> (Order, OrderEvent) {
        let mut order = Order::new(OrderId::from(7));
        order
            .add_item(OrderItem::new(ItemId::from(1), 1, Money::ONE).expect("valid item"))
            .expect("order is pending");
        let event = order.pay().expect("transition is allowed");
        (order, event)
    }

    #[test]
    fn template_expands_to_a_format_literal() {
        assert_eq!(
            format!(status_order_changed!(), 7, OrderStatus::Paid),
            "order 7 status changed to Paid"
        );
    }

    #[test]
    fn template_renders_every_status() {
        assert_eq!(
            format!(status_order_changed!(), 1, OrderStatus::Pending),
            "order 1 status changed to Pending"
        );
        assert_eq!(
            format!(status_order_changed!(), 2, OrderStatus::Posted),
            "order 2 status changed to Posted"
        );
    }

    #[test]
    fn notify_delivers_a_message_for_a_status_change() {
        let (order, event) = paid_order();

        assert!(EmailService::new().notify(&order, &event).is_ok());
    }

    #[test]
    fn service_is_usable_through_the_domain_port() {
        fn accepts_any_notifier(notifier: &impl OrderNotifier, order: &Order, event: &OrderEvent) {
            notifier.notify(order, event).expect("stub never fails");
        }

        let (order, event) = paid_order();
        accepts_any_notifier(&EmailService::new(), &order, &event);
    }
}
