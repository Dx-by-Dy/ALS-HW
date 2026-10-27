//! Командная сторона CQRS — единственная точка входа для изменений.
//!
//! [`spawn`] запускает актор: задачу tokio, которая единолично владеет `TicketService`
//! и выполняет команды строго по одной в порядке поступления. Снаружи с ним говорят через
//! [`CommandBus`]: отправить [`Command`] и дождаться [`Committed`] или подписаться на поток
//! событий. Каждое записанное событие получает глобальную [`Version`] и уходит подписчикам.

mod actor;
mod bus;
mod command;

pub use actor::spawn;
pub use bus::{CommandBus, CommandError, Committed};
pub use command::Command;
pub use ticketing::core::envelope::{Envelope, Version};
pub use ticketing::domain::DomainEvent;
