use crate::event_sourcing::event_store::{EventPos, StreamId};

/// Хранилище снапшотов. Асинхронное по тем же причинам, что и [`EventStore`](crate::event_sourcing::event_store::EventStore).
pub trait SnapshotStore<A> {
    type Error;

    /// `state` — состояние агрегата после первых `pos` событий стрима (см. [`EventPos`]).
    /// Восстановление: `state.recover(store.event_stream_from_pos(id, pos).await?)`.
    fn save_snapshot(
        &mut self,
        id: StreamId,
        pos: EventPos,
        state: A,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn load_latest_snapshot(
        &self,
        id: StreamId,
    ) -> impl Future<Output = Result<Option<(A, EventPos)>, Self::Error>> + Send;
}
