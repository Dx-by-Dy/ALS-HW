use super::envelope::Committed;

/// Входная точка записывающей стороны: принимает команду, выполняет её и сообщает,
/// какие события записаны.
///
/// В отличие от [`CommandHandler`](crate::event_sourcing::command_handler::CommandHandler),
/// который решает судьбу команды внутри одного агрегата, шина работает на уровне приложения:
/// находит нужный агрегат, подставляет время и идентификаторы, упорядочивает команды.
pub trait CommandBus<C> {
    type Event;
    type Error;

    fn send(
        &self,
        command: C,
    ) -> impl Future<Output = Result<Committed<Self::Event>, Self::Error>> + Send;
}
