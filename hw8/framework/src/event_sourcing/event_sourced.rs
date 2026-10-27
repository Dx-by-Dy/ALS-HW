use super::{command_handler::CommandHandler, event_handler::EventHandler};

/// Агрегат, состояние которого восстанавливается из событий.
///
/// Реализуется автоматически для всего, что умеет и решать, и применять события.
pub trait EventSourced: EventHandler + CommandHandler {
    /// Свёртка стрима: применяет события по порядку, начиная с `self`.
    fn recover(self, stream: impl IntoIterator<Item = Self::Event>) -> Self {
        stream.into_iter().fold(self, |s, e| s.apply_event(&e))
    }
}

impl<T: CommandHandler + EventHandler> EventSourced for T {}
