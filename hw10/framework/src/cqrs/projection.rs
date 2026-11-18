use super::envelope::{Envelope, Version};

/// Проекция строит read-модель из потока событий.
///
/// Методы принимают `&self`: одну и ту же модель одновременно обновляет проектор
/// и читают обработчики запросов, поэтому синхронизация — забота реализации.
pub trait Projection<E> {
    type Error;

    /// Применяет событие. События приходят по возрастанию версий без пропусков.
    fn project(&self, event: &Envelope<E>) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// До какой версии модель догнала поток. С неё же проекцию можно продолжить после остановки.
    fn version(&self) -> impl Future<Output = Version> + Send;
}
