use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

/// Часы для демонстрации сроков: идут с настоящего момента, но в `scale` раз быстрее.
///
/// Домен и сага об этом не знают: для них проходят обычные 10 дней. Отсчёт идёт по монотонному
/// времени, поэтому смена системных часов во время работы его не сбивает.
#[derive(Debug, Clone, Copy)]
pub struct ScaledClock {
    start: Timestamp,
    started: Instant,
    scale: u32,
}

impl ScaledClock {
    pub fn starting_now(scale: u32) -> Self {
        Self {
            start: SystemClock.now(),
            started: Instant::now(),
            scale,
        }
    }

    /// Который час на этих часах, когда с запуска прошло `elapsed` настоящего времени.
    fn at(&self, elapsed: Duration) -> Timestamp {
        let secs = elapsed.as_millis().saturating_mul(u128::from(self.scale)) / 1_000;
        self.start
            .plus_secs(u64::try_from(secs).unwrap_or(u64::MAX))
    }
}

impl Clock for ScaledClock {
    fn now(&self) -> Timestamp {
        self.at(self.started.elapsed())
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

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u32 = 86_400;

    #[test]
    fn scaled_clock_starts_from_real_time() {
        let before = SystemClock.now();
        let clock = ScaledClock::starting_now(DAY);
        assert!(clock.at(Duration::ZERO) >= before);
        assert!(clock.at(Duration::ZERO) <= SystemClock.now());
    }

    #[test]
    fn scaled_clock_runs_scale_times_faster() {
        let clock = ScaledClock::starting_now(DAY);
        let start = clock.at(Duration::ZERO);
        assert_eq!(clock.at(Duration::from_secs(10)).secs_since(start), 864_000);
        // Доли секунды тоже идут в счёт: иначе часы шагали бы сразу на сутки.
        assert_eq!(
            clock.at(Duration::from_millis(500)).secs_since(start),
            43_200
        );
    }

    #[test]
    fn scale_of_one_is_real_time() {
        let clock = ScaledClock::starting_now(1);
        let start = clock.at(Duration::ZERO);
        assert_eq!(clock.at(Duration::from_secs(90)).secs_since(start), 90);
    }

    #[test]
    fn huge_elapsed_time_saturates_instead_of_wrapping() {
        let clock = ScaledClock::starting_now(u32::MAX);
        assert_eq!(clock.at(Duration::MAX), Timestamp(u64::MAX));
    }
}
