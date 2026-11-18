//! Команды, которые приходят в сервис снаружи.
//!
//! В отличие от команд агрегатов, здесь нет ни сгенерированных идентификаторов, ни времени,
//! ни данных других агрегатов: всё это подставляет `TicketService`.

use crate::domain::{
    partner::PartnerId,
    tenant::TenantId,
    ticket::{CloseReason, ReminderKind, ResourceAvailability, TicketId},
    user::{Role, UserId},
};
use kernel::sla::CustomerTier;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterTenantCommand {
    pub tenant_name: String,
}

impl RegisterTenantCommand {
    pub fn new(tenant_name: impl Into<String>) -> Self {
        Self {
            tenant_name: tenant_name.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterUserCommand {
    pub user_name: String,
    pub role: Role,
}

impl RegisterUserCommand {
    pub fn new(user_name: impl Into<String>, role: Role) -> Self {
        Self {
            user_name: user_name.into(),
            role,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTicketCommand {
    pub category: String,
    pub description: String,
    pub tenant_id: TenantId,
    /// Уровень клиента по данным CRM: его узнают до того, как команда попадёт в сервис,
    /// чтобы командная сторона не ждала внешних систем.
    pub tier: CustomerTier,
}

impl CreateTicketCommand {
    pub fn new(
        category: impl Into<String>,
        description: impl Into<String>,
        tenant_id: TenantId,
        tier: CustomerTier,
    ) -> Self {
        Self {
            category: category.into(),
            description: description.into(),
            tenant_id,
            tier,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssignAgentCommand {
    pub ticket_id: TicketId,
    pub agent_id: UserId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseTicketCommand {
    pub ticket_id: TicketId,
    pub reason: CloseReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReopenTicketCommand {
    pub ticket_id: TicketId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendReminderCommand {
    pub ticket_id: TicketId,
    pub kind: ReminderKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordResourceCheckCommand {
    pub ticket_id: TicketId,
    pub availability: ResourceAvailability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandToPartnerCommand {
    pub ticket_id: TicketId,
    pub partner_id: PartnerId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportPartnerProgressCommand {
    pub ticket_id: TicketId,
    pub partner_id: PartnerId,
    pub comment: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolveByPartnerCommand {
    pub ticket_id: TicketId,
    pub partner_id: PartnerId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyDiscountCommand {
    pub tenant_id: TenantId,
}
