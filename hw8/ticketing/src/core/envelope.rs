use std::fmt;

/// Глобальная версия — номер события в общем порядке записи всех стримов.
///
/// По аналогии с [`EventPos`](crate::core::event_store::EventPos) это граница *после* события:
/// `Version(0)` — ещё ничего не записано, первое событие получает `Version(1)`.
/// Читатель, который видел всё до версии `n`, просит изменения «после `n`».
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub u64);

impl Version {
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// Событие вместе с его глобальной версией — то, что командная сторона отдаёт подписчикам.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope<E> {
    pub version: Version,
    pub event: E,
}
