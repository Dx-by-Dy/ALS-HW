use crate::core::aggregate::Aggregate;

pub trait EventHandler: Aggregate {
    fn apply_event(self, event: &Self::Event) -> Self;
}
