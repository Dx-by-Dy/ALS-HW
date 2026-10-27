use super::event_store::{EventPos, StreamId};

/// Хранилище снапшотов — сохранённых состояний агрегата на некоторой позиции стрима.
///
/// Снапшот лишь ускоряет восстановление: состояние из снапшота плюс события после него
/// должно совпадать с полной свёрткой стрима. Асинхронное по тем же причинам,
/// что и [`EventStore`](super::event_store::EventStore).
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

    /// Последний сохранённый снапшот стрима; `None`, если снапшотов ещё не было.
    fn load_latest_snapshot(
        &self,
        id: StreamId,
    ) -> impl Future<Output = Result<Option<(A, EventPos)>, Self::Error>> + Send;
}
