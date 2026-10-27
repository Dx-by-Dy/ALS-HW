use crate::event_sourcing::aggregate::Aggregate;

pub trait CommandHandler: Aggregate {
    type Error;

    fn decide(&self, command: &Self::Command) -> Result<Self::Event, Self::Error>;
}
