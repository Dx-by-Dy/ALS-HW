//! Запросы к read-модели. Каждый запрос — структура с параметрами и типом ответа.

use framework::cqrs::{envelope::Version, query::Query};
use ticketing::domain::{
    tenant::TenantId,
    ticket::{TicketId, TicketStatus},
    time::Timestamp,
    user::UserId,
};

use crate::views::{Changes, TenantView, TicketView, UserView};

/// Тикет по номеру.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GetTicket(pub TicketId);

/// Тикеты по возрастанию номера; пустой фильтр — все тикеты.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ListTickets {
    pub status: Option<TicketStatus>,
    pub tenant_id: Option<TenantId>,
}

/// Незакрытые тикеты, назначенные на агента.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveTicketsOfAgent(pub UserId);

/// Открытые тикеты тенанта, которые ещё никто не взял в работу.
/// В домене нет черновиков — это ближайший аналог: тикет заведён, но работа не начата.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnassignedTicketsOfTenant(pub TenantId);

/// Тикеты, закрытые в момент `since` или позже, по времени закрытия.
/// «За последние 7 дней» — это `since = now - 7 дней`: текущее время задаёт вызывающий.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TicketsClosedSince(pub Timestamp);

/// Тенант по номеру.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GetTenant(pub TenantId);

/// Все тенанты по возрастанию номера.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListTenants;

/// Все пользователи по возрастанию номера.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListUsers;

/// Записи, изменённые после версии. `ChangesSince(Version(0))` — всё состояние целиком.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangesSince(pub Version);

impl Query for GetTicket {
    type Output = Option<TicketView>;
}

impl Query for ListTickets {
    type Output = Vec<TicketView>;
}

impl Query for ActiveTicketsOfAgent {
    type Output = Vec<TicketView>;
}

impl Query for UnassignedTicketsOfTenant {
    type Output = Vec<TicketView>;
}

impl Query for TicketsClosedSince {
    type Output = Vec<TicketView>;
}

impl Query for GetTenant {
    type Output = Option<TenantView>;
}

impl Query for ListTenants {
    type Output = Vec<TenantView>;
}

impl Query for ListUsers {
    type Output = Vec<UserView>;
}

impl Query for ChangesSince {
    type Output = Changes;
}
