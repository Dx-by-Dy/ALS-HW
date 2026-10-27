//! CQRS: команды меняют состояние и ничего не читают, запросы читают и ничего не меняют.
//!
//! Запись: [`CommandBus`](command_bus::CommandBus) принимает команду и возвращает
//! [`Committed`](envelope::Committed) — записанные события с глобальными версиями.
//! Чтение: [`Projection`](projection::Projection) строит read-модель из потока событий
//! [`EventSource`](event_source::EventSource), а [`QueryHandler`](query::QueryHandler) отвечает
//! на [`Query`](query::Query) по этой модели. Реакции на события, которые порождают новые
//! команды, описывает [`Policy`](policy::Policy). Фоновые циклы, которые соединяют
//! источник событий с проекциями и политиками, — в [`runner`].

pub mod command_bus;
pub mod envelope;
pub mod event_source;
pub mod policy;
pub mod projection;
pub mod query;
pub mod runner;
