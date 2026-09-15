use super::OrderId;
use super::order_status::OrderStatus;

/// Факт, случившийся с агрегатом.
///
/// `#[must_use]` не даёт потерять событие
#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderEvent {
    StatusChanged {
        id: OrderId,
        from: OrderStatus,
        to: OrderStatus,
    },
}
