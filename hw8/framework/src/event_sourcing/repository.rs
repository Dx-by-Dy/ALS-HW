use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroUsize;

use crate::event_sourcing::{
    event_sourced::EventSourced,
    event_store::{EventPos, EventStore, StreamId},
    snapshot_store::SnapshotStore,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryError<D, S> {
    /// Агрегат отклонил команду.
    Domain(D),
    Storage(S),
}

impl<D: fmt::Display, S: fmt::Display> fmt::Display for RepositoryError<D, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Domain(e) => e.fmt(f),
            Self::Storage(e) => write!(f, "storage error: {e}"),
        }
    }
}

impl<D: Error, S: Error> Error for RepositoryError<D, S> {}

/// Связывает агрегат с хранилищами: восстанавливает состояние из снапшота и событий,
/// выполняет команду и дописывает получившееся событие в стрим.
///
/// Начальное состояние пустого стрима — `A::default()`.
#[derive(Debug)]
pub struct Repository<A, ES, SS> {
    events: ES,
    snapshots: SS,
    /// Снапшот снимается, когда длина стрима кратна этому числу.
    snapshot_every: NonZeroUsize,
    _aggregate: PhantomData<A>,
}

impl<A, ES, SS> Repository<A, ES, SS>
where
    A: EventSourced + Default + Clone,
    A::Event: Clone,
    ES: EventStore<A::Event>,
    SS: SnapshotStore<A, Error = ES::Error>,
{
    pub fn new(events: ES, snapshots: SS, snapshot_every: NonZeroUsize) -> Self {
        Self {
            events,
            snapshots,
            snapshot_every,
            _aggregate: PhantomData,
        }
    }

    /// Текущее состояние: последний снапшот плюс события после него.
    pub async fn load(&self, id: StreamId) -> Result<A, ES::Error> {
        let (state, pos) = self
            .snapshots
            .load_latest_snapshot(id)
            .await?
            .unwrap_or_else(|| (A::default(), EventPos(0)));
        Ok(state.recover(self.events.event_stream_from_pos(id, pos).await?))
    }

    /// load → decide → store (→ snapshot). Возвращает записанное событие.
    pub async fn execute(
        &mut self,
        id: StreamId,
        command: &A::Command,
    ) -> Result<A::Event, RepositoryError<A::Error, ES::Error>> {
        let state = self.load(id).await.map_err(RepositoryError::Storage)?;
        let event = state.decide(command).map_err(RepositoryError::Domain)?;
        let pos = self
            .events
            .store_event(id, event.clone())
            .await
            .map_err(RepositoryError::Storage)?;
        if pos.0 % self.snapshot_every.get() == 0 {
            let state = state.apply_event(&event);
            self.snapshots
                .save_snapshot(id, pos, state)
                .await
                .map_err(RepositoryError::Storage)?;
        }
        Ok(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_sourcing::{
        aggregate::Aggregate, command_handler::CommandHandler, event_handler::EventHandler,
    };
    use crate::in_memory::{
        error::StorageError, event_store::InMemoryEventStore, snapshot_store::InMemorySnapshotStore,
    };

    /// Простейший агрегат: счётчик, который не может уйти в минус.
    #[derive(Debug, Clone, Default, PartialEq)]
    struct Counter(i64);

    impl Aggregate for Counter {
        type Event = i64;
        type Command = i64;
    }

    impl CommandHandler for Counter {
        type Error = &'static str;

        fn decide(&self, delta: &i64) -> Result<i64, Self::Error> {
            if self.0 + delta < 0 {
                Err("negative")
            } else {
                Ok(*delta)
            }
        }
    }

    impl EventHandler for Counter {
        fn apply_event(self, delta: &i64) -> Self {
            Counter(self.0 + delta)
        }
    }

    type CounterRepository =
        Repository<Counter, InMemoryEventStore<i64>, InMemorySnapshotStore<Counter>>;

    const ID: StreamId = StreamId(1);

    fn repository(snapshot_every: usize) -> CounterRepository {
        Repository::new(
            InMemoryEventStore::default(),
            InMemorySnapshotStore::default(),
            NonZeroUsize::new(snapshot_every).unwrap(),
        )
    }

    async fn full_replay(repo: &CounterRepository) -> Counter {
        let events = repo.events.event_stream_from_pos(ID, EventPos(0)).await;
        Counter::default().recover(events.unwrap())
    }

    #[tokio::test]
    async fn empty_stream_loads_default_state() {
        assert_eq!(repository(2).load(ID).await, Ok(Counter(0)));
    }

    #[tokio::test]
    async fn executed_command_is_stored_and_visible_on_load() {
        let mut repo = repository(10);
        assert_eq!(repo.execute(ID, &5).await, Ok(5));
        assert_eq!(repo.execute(ID, &-2).await, Ok(-2));
        assert_eq!(repo.load(ID).await, Ok(Counter(3)));
    }

    #[tokio::test]
    async fn rejected_command_stores_nothing() {
        let mut repo = repository(10);
        repo.execute(ID, &1).await.unwrap();
        assert_eq!(
            repo.execute(ID, &-5).await,
            Err(RepositoryError::<_, StorageError>::Domain("negative"))
        );
        assert_eq!(repo.load(ID).await, Ok(Counter(1)));
        assert_eq!(full_replay(&repo).await, Counter(1));
    }

    #[tokio::test]
    async fn snapshot_is_taken_when_stream_length_is_multiple_of_interval() {
        let mut repo = repository(2);
        repo.execute(ID, &1).await.unwrap();
        assert_eq!(repo.snapshots.load_latest_snapshot(ID).await, Ok(None));
        repo.execute(ID, &2).await.unwrap();
        assert_eq!(
            repo.snapshots.load_latest_snapshot(ID).await,
            Ok(Some((Counter(3), EventPos(2))))
        );
        repo.execute(ID, &3).await.unwrap();
        repo.execute(ID, &4).await.unwrap();
        assert_eq!(
            repo.snapshots.load_latest_snapshot(ID).await,
            Ok(Some((Counter(10), EventPos(4))))
        );
    }

    #[tokio::test]
    async fn load_from_snapshot_equals_full_replay() {
        let mut repo = repository(2);
        for delta in [1, 2, 3, -4, 5] {
            repo.execute(ID, &delta).await.unwrap();
        }
        assert_eq!(repo.load(ID).await.unwrap(), full_replay(&repo).await);
    }

    #[tokio::test]
    async fn load_replays_only_events_after_snapshot() {
        let mut repo = repository(100);
        for delta in [1, 2, 3] {
            repo.execute(ID, &delta).await.unwrap();
        }
        // Подменяем снапшот на позиции 1: если load его использует,
        // то поверх 100 применятся только события 2 и 3.
        repo.snapshots
            .save_snapshot(ID, EventPos(1), Counter(100))
            .await
            .unwrap();
        assert_eq!(repo.load(ID).await, Ok(Counter(105)));
    }
}
