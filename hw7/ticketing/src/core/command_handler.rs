use crate::core::aggregate::Aggregate;

pub trait CommandHandler: Aggregate {
    type Error;

    fn decide(&self, command: &Self::Command) -> Result<Self::Event, Self::Error>;
}
