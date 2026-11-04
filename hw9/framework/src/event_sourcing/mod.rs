//! Event sourcing: состояние агрегата не хранится, а выводится из его событий.
//!
//! Агрегат описывается двумя чистыми функциями:
//! [`CommandHandler::decide`](command_handler::CommandHandler::decide) — по состоянию и команде
//! решает, какое событие произошло (или почему команда отклонена), и
//! [`EventHandler::apply_event`](event_handler::EventHandler::apply_event) — по состоянию
//! и событию строит следующее состояние. Вместе они дают
//! [`EventSourced`](event_sourced::EventSourced): восстановление состояния свёрткой стрима.
//!
//! Хранение: [`EventStore`](event_store::EventStore) — append-only стримы событий,
//! [`SnapshotStore`](snapshot_store::SnapshotStore) — снапшоты, чтобы не сворачивать стрим
//! с самого начала. [`Repository`](repository::Repository) связывает агрегат с хранилищами:
//! load → decide → store. [`Lifecycle`](lifecycle::Lifecycle) описывает частый случай —
//! агрегат, которого до первого события ещё нет.

pub mod aggregate;
pub mod command_handler;
pub mod event_handler;
pub mod event_sourced;
pub mod event_store;
pub mod lifecycle;
pub mod repository;
pub mod snapshot_store;
