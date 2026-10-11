use std::fmt;

use super::{CloseReason, TicketId};
use crate::domain::{tenant::TenantId, time::Timestamp, user::UserId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TicketEvent {
    Created(TicketCreatedEvent),
    AgentAssigned(AgentAssignedEvent),
    Closed(TicketClosedEvent),
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
    pub reason: CloseReason,
    pub at: Timestamp,
}

impl TicketEvent {
    pub fn ticket_id(&self) -> TicketId {
        match self {
            Self::Created(e) => e.ticket_id,
            Self::AgentAssigned(e) => e.ticket_id,
            Self::Closed(e) => e.ticket_id,
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
        }
    }
}
