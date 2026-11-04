use std::fmt;

use super::{CloseReason, ReminderKind, TicketId};
use crate::domain::{tenant::TenantId, time::Timestamp, user::UserId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TicketEvent {
    Created(TicketCreatedEvent),
    AgentAssigned(AgentAssignedEvent),
    Closed(TicketClosedEvent),
    Reopened(TicketReopenedEvent),
    ReminderSent(ReminderSentEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketCreatedEvent {
    pub ticket_id: TicketId,
    pub category: String,
    pub description: String,
    pub tenant_id: TenantId,
    pub at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAssignedEvent {
    pub ticket_id: TicketId,
    pub agent_id: UserId,
    /// Агент, который был назначен до этого, если тикет переназначили.
    pub previous_agent_id: Option<UserId>,
    pub at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketClosedEvent {
    pub ticket_id: TicketId,
    /// Нужен тем, кто следит за тикетами тенанта: проекциям и политике скидок.
    pub tenant_id: TenantId,
    /// Агент, который вёл тикет: его нужно предупредить о закрытии.
    pub agent_id: Option<UserId>,
    pub reason: CloseReason,
    pub at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketReopenedEvent {
    pub ticket_id: TicketId,
    /// Нужен тем, кто следит за тикетами тенанта: проекциям и политике скидок.
    pub tenant_id: TenantId,
    pub at: Timestamp,
}

/// Напоминание о простаивающем тикете. Это не активность: отсчёт неактивности оно не сбрасывает,
/// иначе тикет с напоминаниями никогда бы не закрылся.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderSentEvent {
    pub ticket_id: TicketId,
    /// Кому напомнить; `None` — агента нет, и напоминание адресовано администраторам.
    pub agent_id: Option<UserId>,
    pub kind: ReminderKind,
    pub at: Timestamp,
}

impl TicketEvent {
    pub fn ticket_id(&self) -> TicketId {
        match self {
            Self::Created(e) => e.ticket_id,
            Self::AgentAssigned(e) => e.ticket_id,
            Self::Closed(e) => e.ticket_id,
            Self::Reopened(e) => e.ticket_id,
            Self::ReminderSent(e) => e.ticket_id,
        }
    }
}

impl fmt::Display for TicketEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Created(e) => write!(
                f,
                "TicketCreated: {} [{}] '{}' for {} at {}",
                e.ticket_id, e.category, e.description, e.tenant_id, e.at
            ),
            Self::AgentAssigned(e) => {
                write!(f, "AgentAssigned: {} -> {}", e.ticket_id, e.agent_id)?;
                if let Some(previous) = e.previous_agent_id {
                    write!(f, " (was {previous})")?;
                }
                write!(f, " at {}", e.at)
            }
            Self::Closed(e) => {
                write!(
                    f,
                    "TicketClosed: {} ({}) at {}",
                    e.ticket_id, e.reason, e.at
                )
            }
            Self::Reopened(e) => write!(f, "TicketReopened: {} at {}", e.ticket_id, e.at),
            Self::ReminderSent(e) => {
                write!(f, "ReminderSent: {} ({}) to ", e.ticket_id, e.kind)?;
                match e.agent_id {
                    Some(agent) => write!(f, "{agent}")?,
                    None => f.write_str("admins")?,
                }
                write!(f, " at {}", e.at)
            }
        }
    }
}
