use std::cell::Cell;
use std::rc::Rc;

use crate::domain::time::Timestamp;

pub trait Clock {
    fn now(&self) -> Timestamp;
}

/// Часы, которые двигает вызывающий код. Клоны делят одно время:
/// один клон отдаём сервису, другим управляем в демо и тестах.
#[derive(Debug, Clone, Default)]
pub struct ManualClock(Rc<Cell<Timestamp>>);

impl ManualClock {
    pub fn advance(&self, secs: u64) {
        self.0.set(Timestamp(self.0.get().0 + secs));
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        self.0.get()
    }
}
