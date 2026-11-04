use std::fmt;

use super::{CLOSING_SOON_REMINDER_AFTER_SECS, STALE_REMINDER_AFTER_SECS, TicketId};
use crate::domain::{tenant::TenantId, time::Timestamp, user::User};

/// Команды агрегата `Ticket`.
///
/// Всё, что нужно для решения, уже лежит в команде: идентификатор, время и данные
/// других агрегатов подставляет сервис приложения, поэтому `decide` остаётся чистой.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TicketCommand {
    Create {
        ticket_id: TicketId,
        category: String,
        description: String,
        tenant_id: TenantId,
        at: Timestamp,
    },
    AssignAgent {
        agent: User,
        at: Timestamp,
    },
    Close {
        reason: CloseReason,
        at: Timestamp,
    },
    /// Снова открыть закрытый тикет.
    Reopen {
        at: Timestamp,
    },
    /// Напомнить о тикете, который простаивает.
    SendReminder {
        kind: ReminderKind,
        at: Timestamp,
    },
}

/// Вид напоминания. Варианты идут в том порядке, в каком наступают их сроки.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReminderKind {
    /// Тикет простаивает уже [`STALE_REMINDER_AFTER_SECS`].
    Stale,
    /// До закрытия по неактивности остался последний день.
    ClosingSoon,
}

impl ReminderKind {
    /// Сколько тикет должен простоять без активности, чтобы напоминание стало уместным.
    pub const fn due_after_secs(self) -> u64 {
        match self {
            Self::Stale => STALE_REMINDER_AFTER_SECS,
            Self::ClosingSoon => CLOSING_SOON_REMINDER_AFTER_SECS,
        }
    }
}

impl fmt::Display for ReminderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Stale => "stale",
            Self::ClosingSoon => "closing soon",
        };
        f.write_str(name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// Проблема решена.
    Resolved,
    /// Тикет простоял без активности дольше [`super::INACTIVITY_TIMEOUT_SECS`].
    Inactivity,
}

impl fmt::Display for CloseReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Resolved => "resolved",
            Self::Inactivity => "inactivity",
        };
        f.write_str(name)
    }
}
