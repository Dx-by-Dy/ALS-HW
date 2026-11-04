//! Модели представления — данные в том виде, в каком их читают, а не в каком их решают.

use framework::cqrs::envelope::Version;
use ticketing::application::notifications::{NotificationKind, Recipient};
use ticketing::domain::{
    tenant::{DiscountRate, TenantId},
    ticket::{CloseReason, INACTIVITY_TIMEOUT_SECS, TicketId, TicketStatus},
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
    /// Почему тикет закрыли; `None`, пока он открыт.
    pub close_reason: Option<CloseReason>,
    /// Время создания, назначения агента или повторного открытия: от него считаются сроки.
    pub last_activity: Timestamp,
    /// Сколько напоминаний отправлено с момента последней активности.
    pub reminders_sent: u32,
    /// Версия последнего изменения записи.
    pub version: Version,
}

impl TicketView {
    /// Когда тикет закроется сам, если активности больше не будет; `None`, если он уже закрыт.
    pub fn closes_at(&self) -> Option<Timestamp> {
        self.status
            .is_open()
            .then(|| self.last_activity.plus_secs(INACTIVITY_TIMEOUT_SECS))
    }
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

/// Номер уведомления: уведомления только добавляются и не меняются.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NotificationId(pub usize);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationView {
    pub notification_id: NotificationId,
    pub recipient: Recipient,
    pub ticket_id: TicketId,
    pub kind: NotificationKind,
    pub at: Timestamp,
    /// Версия события, которое его породило.
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
    pub notifications: Vec<NotificationView>,
}
