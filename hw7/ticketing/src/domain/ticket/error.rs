use std::error::Error;
use std::fmt;

use super::TicketId;
use crate::domain::user::{Role, UserId};

/// Нарушения инвариантов агрегата `Ticket`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TicketError {
    /// Тикет с таким идентификатором уже создан.
    AlreadyExists(TicketId),
    /// Команда пришла в пустой стрим: тикета нет.
    NotFound,
    /// Описание проблемы не может быть пустым.
    EmptyDescription,
    /// Назначить на тикет можно только пользователя с ролью `Agent`.
    NotAnAgent { user_id: UserId, role: Role },
    /// Этот агент уже назначен на тикет.
    AlreadyAssigned(UserId),
    /// Закрытый тикет больше не меняется.
    Closed(TicketId),
    /// Открыть заново можно только закрытый тикет.
    AlreadyOpen(TicketId),
    /// Закрыть по неактивности можно только тикет, простоявший дольше таймаута.
    StillActive { idle_secs: u64 },
}

impl fmt::Display for TicketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyExists(id) => write!(f, "{id} already exists"),
            Self::NotFound => write!(f, "ticket does not exist"),
            Self::EmptyDescription => write!(f, "ticket description must not be empty"),
            Self::NotAnAgent { user_id, role } => {
                write!(
                    f,
                    "{user_id} has role {role} and cannot be assigned to a ticket"
                )
            }
            Self::AlreadyAssigned(id) => write!(f, "{id} is already assigned to the ticket"),
            Self::Closed(id) => write!(f, "{id} is closed"),
            Self::AlreadyOpen(id) => write!(f, "{id} is already open"),
            Self::StillActive { idle_secs } => {
                write!(
                    f,
                    "ticket was active {idle_secs}s ago and cannot be closed for inactivity"
                )
            }
        }
    }
}

impl Error for TicketError {}
