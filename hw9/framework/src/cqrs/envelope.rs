use std::fmt;

/// Глобальная версия — номер события в общем порядке записи всех стримов.
///
/// По аналогии с [`EventPos`](crate::event_sourcing::event_store::EventPos) это граница *после* события:
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

/// Событие вместе с его глобальной версией — то, что командная сторона отдаёт читающей.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope<E> {
    pub version: Version,
    pub event: E,
}

/// Результат выполненной команды.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Committed<E> {
    /// Версия после команды. Если команда ничего не записала
    /// (например, закрывать по неактивности было нечего), — текущая версия.
    ///
    /// Это и есть consistency token: read-модель, догнавшая эту версию, видит результат команды.
    pub version: Version,
    /// Записанные события в порядке записи.
    pub events: Vec<Envelope<E>>,
}
