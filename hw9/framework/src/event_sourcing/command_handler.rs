use super::aggregate::Aggregate;

/// Принятие решений: команда либо порождает событие, либо отклоняется.
///
/// `decide` — чистая функция без ввода-вывода: всё, что нужно для решения (время,
/// данные других агрегатов), приходит в самой команде. Состояние она не меняет —
/// это делает [`EventHandler`](super::event_handler::EventHandler), когда событие записано.
pub trait CommandHandler: Aggregate {
    type Error;

    fn decide(&self, command: &Self::Command) -> Result<Self::Event, Self::Error>;
}
