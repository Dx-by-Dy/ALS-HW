use std::cell::Cell;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::time::Timestamp;

pub trait Clock {
    fn now(&self) -> Timestamp;
}

/// Настоящие часы: секунды Unix-времени.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is after the Unix epoch");
        Timestamp(since_epoch.as_secs())
    }
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
