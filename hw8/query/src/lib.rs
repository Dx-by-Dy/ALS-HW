//! Запросная сторона CQRS: read-модель, построенная из потока событий командной стороны.
//!
//! [`ReadModel`] реализует трейты `framework::cqrs`: `Projection` — обновляется событиями
//! (обычно через `run_projection`), `QueryHandler` — отвечает на запросы из [`queries`].
//! Данные лежат в памяти в денормализованном виде ([`views`]) вместе с индексами под
//! конкретные запросы, поэтому чтение не восстанавливает агрегаты и не сканирует всё подряд.

mod read_model;
mod state;

pub mod queries;
pub mod views;

pub use read_model::ReadModel;
pub use state::ProjectionError;
