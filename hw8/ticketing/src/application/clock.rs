use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
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
/// Время атомарное, поэтому сервис с такими часами можно отдать в другую задачу.
#[derive(Debug, Clone, Default)]
pub struct ManualClock(Arc<AtomicU64>);

impl ManualClock {
    pub fn advance(&self, secs: u64) {
        self.0.fetch_add(secs, Ordering::Relaxed);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.load(Ordering::Relaxed))
    }
}
