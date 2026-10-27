use std::fmt;

use super::{tenant::TenantEvent, ticket::TicketEvent, user::UserEvent};

/// Событие любого агрегата. У каждого агрегата свой стрим, а подписчикам
/// (проекциям, политикам) нужен один общий поток в порядке записи.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainEvent {
    Ticket(TicketEvent),
    Tenant(TenantEvent),
    User(UserEvent),
}

impl From<TicketEvent> for DomainEvent {
    fn from(event: TicketEvent) -> Self {
        Self::Ticket(event)
    }
}

impl From<TenantEvent> for DomainEvent {
    fn from(event: TenantEvent) -> Self {
        Self::Tenant(event)
    }
}

impl From<UserEvent> for DomainEvent {
    fn from(event: UserEvent) -> Self {
        Self::User(event)
    }
}

impl fmt::Display for DomainEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ticket(e) => e.fmt(f),
            Self::Tenant(e) => e.fmt(f),
            Self::User(e) => e.fmt(f),
        }
    }
}
