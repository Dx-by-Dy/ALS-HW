use std::fmt;

use crate::time::{DAY_SECS, Timestamp};

/// Уровень обслуживания клиента. Его ведёт CRM, а система тикетов выводит из него SLA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CustomerTier {
    Gold,
    Silver,
    Bronze,
}

impl CustomerTier {
    /// За сколько тикет клиента этого уровня должен быть решён.
    pub const fn resolution_secs(self) -> u64 {
        match self {
            Self::Gold => DAY_SECS,
            Self::Silver => 3 * DAY_SECS,
            Self::Bronze => 5 * DAY_SECS,
        }
    }
}

impl fmt::Display for CustomerTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Gold => "Gold",
            Self::Silver => "Silver",
            Self::Bronze => "Bronze",
        };
        f.write_str(name)
    }
}

/// Обязательство решить тикет к сроку: начало отсчёта и отведённое время.
///
/// Текущее время передаётся снаружи, поэтому одно и то же SLA одинаково считают
/// агрегат, read-модель и CRM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sla {
    started_at: Timestamp,
    resolution_secs: u64,
}

impl Sla {
    pub fn new(started_at: Timestamp, resolution_secs: u64) -> Self {
        Self {
            started_at,
            resolution_secs,
        }
    }

    /// SLA для клиента уровня `tier`, отсчёт с `started_at`.
    pub fn for_tier(tier: CustomerTier, started_at: Timestamp) -> Self {
        Self::new(started_at, tier.resolution_secs())
    }

    pub fn started_at(&self) -> Timestamp {
        self.started_at
    }

    pub fn resolution_secs(&self) -> u64 {
        self.resolution_secs
    }

    /// Крайний срок решения. Решение ровно в этот момент ещё укладывается в SLA.
    pub fn due_at(&self) -> Timestamp {
        self.started_at.plus_secs(self.resolution_secs)
    }

    /// Нарушено ли SLA к моменту `at`: срок прошёл, а тикет не решён.
    ///
    /// Для решённого тикета `at` — момент решения, и тогда ответ говорит, уложились ли в срок.
    pub fn is_overdue(&self, at: Timestamp) -> bool {
        at > self.due_at()
    }

    /// Сколько осталось до срока к моменту `at`; ноль, если срок прошёл.
    pub fn remaining_secs(&self, at: Timestamp) -> u64 {
        self.due_at().secs_since(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: Timestamp = Timestamp(1_000);

    #[test]
    fn higher_tier_gets_shorter_resolution_time() {
        assert_eq!(CustomerTier::Gold.resolution_secs(), DAY_SECS);
        assert_eq!(CustomerTier::Silver.resolution_secs(), 3 * DAY_SECS);
        assert_eq!(CustomerTier::Bronze.resolution_secs(), 5 * DAY_SECS);
    }

    #[test]
    fn deadline_is_start_plus_resolution_time() {
        let sla = Sla::for_tier(CustomerTier::Silver, T0);
        assert_eq!(sla.started_at(), T0);
        assert_eq!(sla.resolution_secs(), 3 * DAY_SECS);
        assert_eq!(sla.due_at(), T0.plus_secs(3 * DAY_SECS));
    }

    #[test]
    fn sla_is_overdue_only_after_deadline() {
        let sla = Sla::new(T0, 100);
        assert!(!sla.is_overdue(T0));
        assert!(!sla.is_overdue(T0.plus_secs(99)));
        assert!(
            !sla.is_overdue(T0.plus_secs(100)),
            "deadline itself is in time"
        );
        assert!(sla.is_overdue(T0.plus_secs(101)));
    }

    #[test]
    fn remaining_time_counts_down_to_zero() {
        let sla = Sla::new(T0, 100);
        assert_eq!(sla.remaining_secs(T0), 100);
        assert_eq!(sla.remaining_secs(T0.plus_secs(40)), 60);
        assert_eq!(sla.remaining_secs(T0.plus_secs(100)), 0);
        assert_eq!(sla.remaining_secs(T0.plus_secs(500)), 0);
    }

    #[test]
    fn deadline_near_end_of_time_does_not_overflow() {
        let sla = Sla::new(Timestamp(u64::MAX - 10), DAY_SECS);
        assert_eq!(sla.due_at(), Timestamp(u64::MAX));
        assert!(!sla.is_overdue(Timestamp(u64::MAX)));
    }

    #[test]
    fn tier_is_readable() {
        assert_eq!(CustomerTier::Gold.to_string(), "Gold");
    }
}
