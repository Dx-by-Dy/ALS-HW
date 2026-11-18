//! Перевод моделей представления в DTO контракта `api` и обратно.

use api::{
    ChangesDto, CloseReason, CustomerTier, NotificationDto, NotificationKind, PartnerUpdateDto,
    Recipient, ReminderKind, ResourceAvailability, Role, TenantDto, TicketDto, TicketStatus,
    UserDto,
};
use query::views::{Changes, NotificationView, TenantView, TicketView, UserView};
use ticketing::application::{notifications, partners::PartnerRoster};
use ticketing::domain::{sla, ticket, user};

/// Тикет для внутреннего API. Имя партнёра берётся из справочника `partners`.
pub fn ticket_dto(ticket: &TicketView, partners: &PartnerRoster) -> TicketDto {
    TicketDto {
        ticket_id: ticket.ticket_id.0,
        category: ticket.category.clone(),
        description: ticket.description.clone(),
        status: match ticket.status {
            ticket::TicketStatus::Open => TicketStatus::Open,
            ticket::TicketStatus::InProgress => TicketStatus::InProgress,
            ticket::TicketStatus::Closed => TicketStatus::Closed,
        },
        assigned_agent: ticket.assigned_agent.map(|id| id.0),
        tenant_id: ticket.tenant_id.0,
        created_at: ticket.created_at.0,
        closed_at: ticket.closed_at.map(|at| at.0),
        close_reason: ticket.close_reason.map(close_reason),
        last_activity: ticket.last_activity.0,
        closes_at: ticket.closes_at().map(|at| at.0),
        reminders_sent: ticket.reminders_sent,
        tier: match ticket.tier {
            sla::CustomerTier::Gold => CustomerTier::Gold,
            sla::CustomerTier::Silver => CustomerTier::Silver,
            sla::CustomerTier::Bronze => CustomerTier::Bronze,
        },
        sla_due_at: ticket.sla.due_at().0,
        sla_met: ticket.sla_met,
        resources: ticket.resources.map(|r| match r {
            ticket::ResourceAvailability::Available => ResourceAvailability::Available,
            ticket::ResourceAvailability::Unavailable => ResourceAvailability::Unavailable,
        }),
        partner: ticket.partner.map(|(partner_id, _)| {
            partners
                .get(partner_id)
                .map_or_else(|| partner_id.to_string(), |p| p.name.clone())
        }),
        handed_over_at: ticket.partner.map(|(_, at)| at.0),
        partner_updates: ticket
            .partner_updates
            .iter()
            .map(|u| PartnerUpdateDto {
                comment: u.comment.clone(),
                at: u.at.0,
            })
            .collect(),
    }
}

fn close_reason(reason: ticket::CloseReason) -> CloseReason {
    match reason {
        ticket::CloseReason::Resolved => CloseReason::Resolved,
        ticket::CloseReason::Inactivity => CloseReason::Inactivity,
    }
}

fn reminder_kind(kind: ticket::ReminderKind) -> ReminderKind {
    match kind {
        ticket::ReminderKind::Stale => ReminderKind::Stale,
        ticket::ReminderKind::ClosingSoon => ReminderKind::ClosingSoon,
    }
}

pub fn notification_dto(notification: &NotificationView) -> NotificationDto {
    NotificationDto {
        notification_id: notification.notification_id.0,
        recipient: match notification.recipient {
            notifications::Recipient::Agent(id) => Recipient::Agent(id.0),
            notifications::Recipient::Admins => Recipient::Admins,
        },
        ticket_id: notification.ticket_id.0,
        kind: match notification.kind {
            notifications::NotificationKind::NewTicket => NotificationKind::NewTicket,
            notifications::NotificationKind::Reopened => NotificationKind::Reopened,
            notifications::NotificationKind::Assigned => NotificationKind::Assigned,
            notifications::NotificationKind::Unassigned => NotificationKind::Unassigned,
            notifications::NotificationKind::Reminder(kind) => {
                NotificationKind::Reminder(reminder_kind(kind))
            }
            notifications::NotificationKind::Closed(reason) => {
                NotificationKind::Closed(close_reason(reason))
            }
        },
        at: notification.at.0,
    }
}

pub fn ticket_status(status: TicketStatus) -> ticket::TicketStatus {
    match status {
        TicketStatus::Open => ticket::TicketStatus::Open,
        TicketStatus::InProgress => ticket::TicketStatus::InProgress,
        TicketStatus::Closed => ticket::TicketStatus::Closed,
    }
}

pub fn role(role: Role) -> user::Role {
    match role {
        Role::Customer => user::Role::Customer,
        Role::Agent => user::Role::Agent,
        Role::Admin => user::Role::Admin,
    }
}

pub fn tier(tier: CustomerTier) -> sla::CustomerTier {
    match tier {
        CustomerTier::Gold => sla::CustomerTier::Gold,
        CustomerTier::Silver => sla::CustomerTier::Silver,
        CustomerTier::Bronze => sla::CustomerTier::Bronze,
    }
}

pub fn user_dto(user: &UserView) -> UserDto {
    UserDto {
        user_id: user.user_id.0,
        user_name: user.user_name.clone(),
        role: match user.role {
            user::Role::Customer => Role::Customer,
            user::Role::Agent => Role::Agent,
            user::Role::Admin => Role::Admin,
        },
    }
}

pub fn tenant_dto(tenant: &TenantView) -> TenantDto {
    TenantDto {
        tenant_id: tenant.tenant_id.0,
        tenant_name: tenant.tenant_name.clone(),
        discount_percent: tenant.discount_rate.percent(),
        open_tickets: tenant.open_tickets,
    }
}

pub fn changes_dto(
    epoch: u64,
    full: bool,
    changes: &Changes,
    partners: &PartnerRoster,
) -> ChangesDto {
    ChangesDto {
        epoch,
        version: changes.version.0,
        full,
        tickets: changes
            .tickets
            .iter()
            .map(|t| ticket_dto(t, partners))
            .collect(),
        tenants: changes.tenants.iter().map(tenant_dto).collect(),
        users: changes.users.iter().map(user_dto).collect(),
        notifications: changes.notifications.iter().map(notification_dto).collect(),
    }
}
