//! Фоновые циклы, которые связывают части CQRS между собой. Они написаны только
//! на трейтах, поэтому одинаково работают с любыми реализациями.

use std::error::Error;
use std::fmt;

use super::{
    command_bus::CommandBus,
    envelope::Version,
    event_source::{EventSource, Subscription},
    policy::Policy,
    projection::Projection,
};

/// Почему фоновый цикл остановился с ошибкой.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError<S, P> {
    /// Источник событий не дал подписаться.
    Source(S),
    /// Получатель событий (проекция) не смог обработать событие.
    Sink(P),
}

impl<S: fmt::Display, P: fmt::Display> fmt::Display for RunError<S, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(e) => write!(f, "event source failed: {e}"),
            Self::Sink(e) => write!(f, "event handler failed: {e}"),
        }
    }
}

impl<S: Error, P: Error> Error for RunError<S, P> {}

/// Держит проекцию в актуальном состоянии: подписывается на источник с той версии,
/// до которой проекция уже дошла, и применяет события по мере поступления.
///
/// Завершается `Ok`, когда источник закрывает поток, и ошибкой, если проекция
/// не смогла применить событие: продолжать с дырой в модели нельзя.
pub async fn run_projection<E, S, P>(
    source: &S,
    projection: &P,
) -> Result<(), RunError<S::Error, P::Error>>
where
    S: EventSource<E>,
    P: Projection<E>,
{
    let after = projection.version().await;
    let mut events = source.subscribe(after).await.map_err(RunError::Source)?;
    while let Some(event) = events.next().await {
        projection.project(&event).await.map_err(RunError::Sink)?;
    }
    Ok(())
}

/// Запускает политику: на каждое событие с версией больше `after` отправляет в шину
/// предложенные ею команды — по одной, в порядке событий.
///
/// Отклонённая команда цикл не останавливает: решение принимает агрегат, и отказ для политики —
/// обычный исход. Ошибки уходят в `on_error`, где их можно отфильтровать или записать в журнал.
/// Завершается `Ok`, когда источник закрывает поток.
pub async fn run_policy<E, S, P, B>(
    source: &S,
    after: Version,
    policy: &P,
    bus: &B,
    mut on_error: impl FnMut(B::Error),
) -> Result<(), S::Error>
where
    S: EventSource<E>,
    P: Policy<E>,
    B: CommandBus<P::Command>,
{
    let mut events = source.subscribe(after).await?;
    while let Some(envelope) = events.next().await {
        for command in policy.react(&envelope.event) {
            if let Err(e) = bus.send(command).await {
                on_error(e);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;
    use crate::cqrs::envelope::{Committed, Envelope};

    /// Источник с заранее записанным журналом; поток закрывается, когда журнал кончился.
    struct Journal(Vec<&'static str>);

    struct Replay(VecDeque<Envelope<&'static str>>);

    impl Subscription<&'static str> for Replay {
        async fn next(&mut self) -> Option<Envelope<&'static str>> {
            self.0.pop_front()
        }
    }

    impl EventSource<&'static str> for Journal {
        type Error = &'static str;
        type Subscription = Replay;

        async fn subscribe(&self, after: Version) -> Result<Replay, &'static str> {
            let events = (1..)
                .map(Version)
                .zip(self.0.iter().copied())
                .filter(|(version, _)| *version > after)
                .map(|(version, event)| Envelope { version, event })
                .collect();
            Ok(Replay(events))
        }
    }

    /// Проекция, которая запоминает события и отказывается применять `"bad"`.
    #[derive(Default)]
    struct Recorder(Mutex<(Version, Vec<&'static str>)>);

    impl Projection<&'static str> for Recorder {
        type Error = Version;

        async fn project(&self, event: &Envelope<&'static str>) -> Result<(), Version> {
            if event.event == "bad" {
                return Err(event.version);
            }
            let mut state = self.0.lock().unwrap();
            state.0 = event.version;
            state.1.push(event.event);
            Ok(())
        }

        async fn version(&self) -> Version {
            self.0.lock().unwrap().0
        }
    }

    #[tokio::test]
    async fn projection_continues_from_its_own_version() {
        let projection = Recorder(Mutex::new((Version(1), vec!["a"])));
        let source = Journal(vec!["a", "b", "c"]);
        assert_eq!(run_projection(&source, &projection).await, Ok(()));
        assert_eq!(
            *projection.0.lock().unwrap(),
            (Version(3), vec!["a", "b", "c"])
        );
    }

    #[tokio::test]
    async fn failed_event_stops_the_loop() {
        let projection = Recorder::default();
        let source = Journal(vec!["a", "bad", "c"]);
        assert_eq!(
            run_projection(&source, &projection).await,
            Err(RunError::Sink(Version(2)))
        );
        assert_eq!(*projection.0.lock().unwrap(), (Version(1), vec!["a"]));
    }

    /// Политика: на каждое `"order"` — команда `"bill"`, на `"refund"` — `"bill"` и `"refund"`.
    struct Billing;

    impl Policy<&'static str> for Billing {
        type Command = &'static str;

        fn react(&self, event: &&'static str) -> Vec<&'static str> {
            match *event {
                "order" => vec!["bill"],
                "refund" => vec!["bill", "refund"],
                _ => vec![],
            }
        }
    }

    /// Шина, которая запоминает принятые команды и отклоняет `"refund"`.
    #[derive(Default)]
    struct Bus(Mutex<Vec<&'static str>>);

    impl CommandBus<&'static str> for Bus {
        type Event = &'static str;
        type Error = &'static str;

        async fn send(
            &self,
            command: &'static str,
        ) -> Result<Committed<&'static str>, &'static str> {
            if command == "refund" {
                return Err("refund rejected");
            }
            let mut sent = self.0.lock().unwrap();
            sent.push(command);
            Ok(Committed {
                version: Version(sent.len() as u64),
                events: vec![],
            })
        }
    }

    #[tokio::test]
    async fn policy_commands_are_sent_in_event_order() {
        let source = Journal(vec!["order", "ship", "refund", "order"]);
        let bus = Bus::default();
        let mut errors = Vec::new();
        let result = run_policy(&source, Version(0), &Billing, &bus, |e| errors.push(e)).await;
        assert_eq!(result, Ok(()));
        assert_eq!(*bus.0.lock().unwrap(), ["bill", "bill", "bill"]);
        assert_eq!(errors, ["refund rejected"]);
    }

    #[tokio::test]
    async fn policy_reacts_only_to_events_after_version() {
        let source = Journal(vec!["order", "order", "order"]);
        let bus = Bus::default();
        run_policy(&source, Version(2), &Billing, &bus, |_| {})
            .await
            .unwrap();
        assert_eq!(bus.0.lock().unwrap().len(), 1);
    }
}
