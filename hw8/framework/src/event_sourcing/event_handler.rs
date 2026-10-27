use crate::event_sourcing::aggregate::Aggregate;

pub trait EventHandler: Aggregate {
    fn apply_event(self, event: &Self::Event) -> Self;
}
