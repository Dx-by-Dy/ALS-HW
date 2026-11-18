//! Общие заготовки для тестов фоновых циклов слоя приложения.

use std::collections::VecDeque;

use framework::cqrs::{
    envelope::{Envelope, Version},
    event_source::{EventSource, Subscription},
};

use crate::domain::DomainEvent;

/// Источник с заранее записанным журналом; поток закрывается, когда журнал кончился.
pub struct Journal(pub Vec<DomainEvent>);

pub struct Replay(VecDeque<Envelope<DomainEvent>>);

impl Subscription<DomainEvent> for Replay {
    async fn next(&mut self) -> Option<Envelope<DomainEvent>> {
        self.0.pop_front()
    }
}

impl EventSource<DomainEvent> for Journal {
    type Error = &'static str;
    type Subscription = Replay;

    async fn subscribe(&self, after: Version) -> Result<Replay, &'static str> {
        let events = (1..)
            .map(Version)
            .zip(self.0.iter().cloned())
            .filter(|(version, _)| *version > after)
            .map(|(version, event)| Envelope { version, event })
            .collect();
        Ok(Replay(events))
    }
}
