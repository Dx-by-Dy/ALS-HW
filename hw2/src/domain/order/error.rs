use std::error::Error;
use std::fmt;

use super::order_items::ItemId;
use super::order_status::OrderStatus;

/// Нарушения инвариантов агрегата `Order`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderError {
    /// Переход статуса не разрешён текущим состоянием.
    InvalidTransition { from: OrderStatus, to: OrderStatus },
    /// Оплатить можно только непустой заказ.
    EmptyOrder,
    /// Состав заказа менять нельзя после оплаты.
    ItemsLocked(OrderStatus),
    /// Количество позиции должно быть положительным.
    ZeroQuantity,
    /// Цена позиции не может быть отрицательной.
    NegativePrice,
    /// Позиции с таким идентификатором в заказе нет.
    ItemNotFound(ItemId),
}

impl fmt::Display for OrderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTransition { from, to } => {
                write!(f, "cannot change order status from {from} to {to}")
            }
            Self::EmptyOrder => write!(f, "cannot pay for an order without items"),
            Self::ItemsLocked(status) => write!(f, "order items are locked in status {status}"),
            Self::ZeroQuantity => write!(f, "item quantity must be greater than zero"),
            Self::NegativePrice => write!(f, "item price must not be negative"),
            Self::ItemNotFound(id) => write!(f, "order has no item {id}"),
        }
    }
}

impl Error for OrderError {}
