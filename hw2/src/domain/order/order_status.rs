use std::fmt;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OrderStatus {
    #[default]
    Pending,
    Paid,
    Posted,
}

impl OrderStatus {
    /// Жизненный цикл заказа.
    pub fn can_transition_to(self, next: OrderStatus) -> bool {
        matches!(
            (self, next),
            (OrderStatus::Pending, OrderStatus::Paid) | (OrderStatus::Paid, OrderStatus::Posted)
        )
    }

    pub fn allows_item_changes(self) -> bool {
        matches!(self, OrderStatus::Pending)
    }
}

impl fmt::Display for OrderStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Pending => "Pending",
            Self::Paid => "Paid",
            Self::Posted => "Posted",
        };
        f.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_status_is_pending() {
        assert_eq!(OrderStatus::default(), OrderStatus::Pending);
    }

    #[test]
    fn lifecycle_goes_pending_paid_posted() {
        assert!(OrderStatus::Pending.can_transition_to(OrderStatus::Paid));
        assert!(OrderStatus::Paid.can_transition_to(OrderStatus::Posted));
    }

    #[test]
    fn transitions_are_not_reversible() {
        assert!(!OrderStatus::Paid.can_transition_to(OrderStatus::Pending));
        assert!(!OrderStatus::Posted.can_transition_to(OrderStatus::Paid));
        assert!(!OrderStatus::Posted.can_transition_to(OrderStatus::Pending));
    }

    #[test]
    fn steps_cannot_be_skipped_or_repeated() {
        assert!(!OrderStatus::Pending.can_transition_to(OrderStatus::Posted));
        assert!(!OrderStatus::Paid.can_transition_to(OrderStatus::Paid));
        assert!(!OrderStatus::Pending.can_transition_to(OrderStatus::Pending));
    }

    #[test]
    fn items_are_mutable_only_before_payment() {
        assert!(OrderStatus::Pending.allows_item_changes());
        assert!(!OrderStatus::Paid.allows_item_changes());
        assert!(!OrderStatus::Posted.allows_item_changes());
    }

    #[test]
    fn display_names_match_variants() {
        assert_eq!(OrderStatus::Pending.to_string(), "Pending");
        assert_eq!(OrderStatus::Paid.to_string(), "Paid");
        assert_eq!(OrderStatus::Posted.to_string(), "Posted");
    }
}
