use crate::core::{command_handler::CommandHandler, event_handler::EventHandler};

pub trait EventSourced: EventHandler + CommandHandler {
    fn recover(self, stream: impl IntoIterator<Item = Self::Event>) -> Self {
        stream.into_iter().fold(self, |s, e| s.apply_event(&e))
    }
}

impl<T: CommandHandler + EventHandler> EventSourced for T {}
