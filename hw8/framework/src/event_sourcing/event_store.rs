#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StreamId(pub usize);

/// Позиция в стриме — количество событий, записанных в стрим к этому моменту,
/// то есть граница *после* события, а не номер самого события.
///
/// - `EventPos(0)` — пустой стрим, начальное состояние агрегата.
/// - `EventPos(n)` — состояние после применения первых `n` событий (индексы `0..n`).
///
/// Благодаря этому снапшот на позиции `n` и чтение стрима с позиции `n`
/// стыкуются без ±1: снапшот уже учитывает события `0..n`, а стрим отдаёт `n..`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EventPos(pub usize);

/// Хранилище событий. Методы асинхронные: настоящее хранилище ходит в сеть или на диск,
/// а возвращаемые future — `Send`, чтобы с хранилищем можно было работать из задачи tokio.
pub trait EventStore<E> {
    type Error;

    /// События стрима с индексами `pos..`, в порядке записи.
    ///
    /// Для несуществующего стрима или `pos`, равной текущей длине стрима,
    /// возвращает пустой вектор.
    fn event_stream_from_pos(
        &self,
        stream_id: StreamId,
        pos: EventPos,
    ) -> impl Future<Output = Result<Vec<E>, Self::Error>> + Send;

    /// Дописывает событие в конец стрима и возвращает новую позицию,
    /// то есть длину стрима вместе с этим событием.
    /// Первое событие в стриме вернёт `EventPos(1)`.
    fn store_event(
        &mut self,
        stream_id: StreamId,
        event: E,
    ) -> impl Future<Output = Result<EventPos, Self::Error>> + Send;
}
