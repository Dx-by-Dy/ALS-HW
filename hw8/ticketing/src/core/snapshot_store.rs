use crate::core::event_store::{EventPos, StreamId};

pub trait SnapshotStore<A> {
    type Error;

    /// `state` — состояние агрегата после первых `pos` событий стрима (см. [`EventPos`]).
    /// Восстановление: `state.recover(store.event_stream_from_pos(id, pos)?)`.
    fn save_snapshot(&mut self, id: StreamId, pos: EventPos, state: A) -> Result<(), Self::Error>;
    fn load_latest_snapshot(&self, id: StreamId) -> Result<Option<(A, EventPos)>, Self::Error>;
}
