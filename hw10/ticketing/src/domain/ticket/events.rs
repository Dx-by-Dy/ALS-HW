use std::fmt;

use super::{CloseReason, ReminderKind, ResourceAvailability, TicketId};
use crate::domain::{partner::PartnerId, tenant::TenantId, time::Timestamp, user::UserId};
use kernel::sla::{CustomerTier, Sla};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TicketEvent {
    Created(TicketCreatedEvent),
    AgentAssigned(AgentAssignedEvent),
    Closed(TicketClosedEvent),
    Reopened(TicketReopenedEvent),
    ReminderSent(ReminderSentEvent),
    ResourcesChecked(ResourcesCheckedEvent),
    HandedToPartner(HandedToPartnerEvent),
    PartnerProgressReported(PartnerProgressReportedEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketCreatedEvent {
    pub ticket_id: TicketId,
    pub category: String,
    pub description: String,
    pub tenant_id: TenantId,
    /// Уровень клиента по данным CRM на момент создания.
    pub tier: CustomerTier,
    /// SLA, обещанное клиенту. Записано в событие, чтобы смена правил не меняла прошлых обещаний.
    pub sla: Sla,
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
    /// Уложились ли в SLA: о нём сообщают клиенту.
    pub sla_met: bool,
    pub at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketReopenedEvent {
    pub ticket_id: TicketId,
    /// Нужен тем, кто следит за тикетами тенанта: проекциям и политике скидок.
    pub tenant_id: TenantId,
    /// Новое SLA: открытый заново тикет решают с начала.
    pub sla: Sla,
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

/// Результат проверки ресурсов в ERP. Это не активность: проверку делает система, а не люди.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourcesCheckedEvent {
    pub ticket_id: TicketId,
    pub availability: ResourceAvailability,
    pub at: Timestamp,
}

/// Тикет передан партнёру. Это ещё не активность: партнёр пока ничего не сделал.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandedToPartnerEvent {
    pub ticket_id: TicketId,
    pub partner_id: PartnerId,
    pub at: Timestamp,
}

/// Партнёр сообщил о ходе работы. Это активность: отсчёт неактивности начинается заново.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartnerProgressReportedEvent {
    pub ticket_id: TicketId,
    pub partner_id: PartnerId,
    pub comment: String,
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
            Self::ResourcesChecked(e) => e.ticket_id,
            Self::HandedToPartner(e) => e.ticket_id,
            Self::PartnerProgressReported(e) => e.ticket_id,
        }
    }
}

impl fmt::Display for TicketEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Created(e) => write!(
                f,
                "TicketCreated: {} [{}] '{}' for {} ({}, due {}) at {}",
                e.ticket_id,
                e.category,
                e.description,
                e.tenant_id,
                e.tier,
                e.sla.due_at(),
                e.at
            ),
            Self::AgentAssigned(e) => {
                write!(f, "AgentAssigned: {} -> {}", e.ticket_id, e.agent_id)?;
                if let Some(previous) = e.previous_agent_id {
                    write!(f, " (was {previous})")?;
                }
                write!(f, " at {}", e.at)
            }
            Self::Closed(e) => {
                let sla = if e.sla_met { "SLA met" } else { "SLA missed" };
                write!(
                    f,
                    "TicketClosed: {} ({}, {sla}) at {}",
                    e.ticket_id, e.reason, e.at
                )
            }
            Self::Reopened(e) => write!(
                f,
                "TicketReopened: {} (due {}) at {}",
                e.ticket_id,
                e.sla.due_at(),
                e.at
            ),
            Self::ReminderSent(e) => {
                write!(f, "ReminderSent: {} ({}) to ", e.ticket_id, e.kind)?;
                match e.agent_id {
                    Some(agent) => write!(f, "{agent}")?,
                    None => f.write_str("admins")?,
                }
                write!(f, " at {}", e.at)
            }
            Self::ResourcesChecked(e) => write!(
                f,
                "ResourcesChecked: {} resources {} at {}",
                e.ticket_id, e.availability, e.at
            ),
            Self::HandedToPartner(e) => write!(
                f,
                "HandedToPartner: {} -> {} at {}",
                e.ticket_id, e.partner_id, e.at
            ),
            Self::PartnerProgressReported(e) => write!(
                f,
                "PartnerProgressReported: {} by {} '{}' at {}",
                e.ticket_id, e.partner_id, e.comment, e.at
            ),
        }
    }
}
