//! Перевод моделей представления в DTO контракта `api` и обратно.

use api::{ChangesDto, Role, TenantDto, TicketDto, TicketStatus, UserDto};
use query::views::{Changes, TenantView, TicketView, UserView};
use ticketing::domain::{ticket, user};

pub fn ticket_dto(ticket: &TicketView) -> TicketDto {
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
    }
}

pub fn ticket_status(status: TicketStatus) -> ticket::TicketStatus {
    match status {
        TicketStatus::Open => ticket::TicketStatus::Open,
        TicketStatus::InProgress => ticket::TicketStatus::InProgress,
        TicketStatus::Closed => ticket::TicketStatus::Closed,
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

pub fn changes_dto(epoch: u64, full: bool, changes: &Changes) -> ChangesDto {
    ChangesDto {
        epoch,
        version: changes.version.0,
        full,
        tickets: changes.tickets.iter().map(ticket_dto).collect(),
        tenants: changes.tenants.iter().map(tenant_dto).collect(),
        users: changes.users.iter().map(user_dto).collect(),
    }
}
