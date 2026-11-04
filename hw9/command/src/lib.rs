//! Командная сторона CQRS — единственная точка входа для изменений.
//!
//! [`spawn`] запускает актор: задачу tokio, которая единолично владеет `TicketService`
//! и выполняет команды строго по одной в порядке поступления. Снаружи с ним говорят через
//! [`CommandActorHandle`], который реализует трейты `framework::cqrs`:
//! `CommandBus` — отправить [`Command`] и дождаться `Committed`,
//! `EventSource` — подписаться на поток событий. Каждое записанное событие получает
//! глобальную версию и уходит подписчикам. [`repeat`] превращает время в источник команд.

mod actor;
mod command;
mod handle;
mod schedule;

pub use actor::spawn;
pub use command::Command;
pub use handle::{CommandActorHandle, CommandError, EventSubscription};
pub use schedule::repeat;
