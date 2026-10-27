//! Модели представления — данные в том виде, в каком их читают, а не в каком их решают.

use framework::cqrs::envelope::Version;
use ticketing::domain::{
    tenant::{DiscountRate, TenantId},
    ticket::{TicketId, TicketStatus},
    time::Timestamp,
    user::{Role, UserId},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketView {
    pub ticket_id: TicketId,
    pub category: String,
    pub description: String,
    pub status: TicketStatus,
    /// У закрытого тикета остаётся агент, который им занимался.
    pub assigned_agent: Option<UserId>,
    pub tenant_id: TenantId,
    pub created_at: Timestamp,
    /// Когда тикет закрыли; `None`, пока он открыт.
    pub closed_at: Option<Timestamp>,
    /// Версия последнего изменения записи.
    pub version: Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantView {
    pub tenant_id: TenantId,
    pub tenant_name: String,
    pub discount_rate: DiscountRate,
    /// Число открытых тикетов тенанта; поддерживается проекцией по событиям тикетов.
    pub open_tickets: usize,
    pub version: Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserView {
    pub user_id: UserId,
    pub user_name: String,
    pub role: Role,
    pub version: Version,
}

/// Всё, что изменилось после некоторой версии: каждая запись в последнем состоянии,
/// по одному разу, в порядке изменения.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// Версия модели, на которой собран ответ: с неё стоит просить следующие изменения.
    pub version: Version,
    pub tickets: Vec<TicketView>,
    pub tenants: Vec<TenantView>,
    pub users: Vec<UserView>,
}
