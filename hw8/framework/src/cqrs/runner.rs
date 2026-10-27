//! Фоновые циклы, которые связывают части CQRS между собой. Они написаны только
//! на трейтах, поэтому одинаково работают с любыми реализациями.

use std::error::Error;
use std::fmt;

use super::{
    event_source::{EventSource, Subscription},
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

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;
    use crate::cqrs::envelope::{Envelope, Version};

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
}
