/// Агрегат — граница согласованности: команды к нему решаются по его собственному
/// состоянию, а изменения записываются в его собственный стрим.
///
/// Сам трейт задаёт только словарь агрегата — типы команд и событий; поведение
/// описывают [`CommandHandler`](super::command_handler::CommandHandler)
/// и [`EventHandler`](super::event_handler::EventHandler).
pub trait Aggregate: Sized {
    type Event;
    type Command;
}
