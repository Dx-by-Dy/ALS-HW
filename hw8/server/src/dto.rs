//! Перевод агрегатов в DTO контракта `api`.

use api::{Role, TenantDto, TicketDto, TicketStatus, UserDto};
use ticketing::domain::{tenant::Tenant, ticket, ticket::Ticket, user, user::User};

pub fn ticket_dto(ticket: &Ticket) -> TicketDto {
    TicketDto {
        ticket_id: ticket.ticket_id().0,
        category: ticket.category().to_owned(),
        description: ticket.description().to_owned(),
        status: match ticket.status() {
            ticket::TicketStatus::Open => TicketStatus::Open,
            ticket::TicketStatus::InProgress => TicketStatus::InProgress,
            ticket::TicketStatus::Closed => TicketStatus::Closed,
        },
        assigned_agent: ticket.assigned_agent().map(|id| id.0),
        tenant_id: ticket.tenant().0,
    }
}

pub fn user_dto(user: &User) -> UserDto {
    UserDto {
        user_id: user.user_id().0,
        user_name: user.user_name().to_owned(),
        role: match user.role() {
            user::Role::Customer => Role::Customer,
            user::Role::Agent => Role::Agent,
            user::Role::Admin => Role::Admin,
        },
    }
}

pub fn tenant_dto(tenant: &Tenant, open_tickets: usize) -> TenantDto {
    TenantDto {
        tenant_id: tenant.tenant_id().0,
        tenant_name: tenant.tenant_name().to_owned(),
        discount_percent: tenant.discount_rate().percent(),
        open_tickets,
    }
}
