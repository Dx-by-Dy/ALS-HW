use std::fmt;

use super::TicketId;
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
