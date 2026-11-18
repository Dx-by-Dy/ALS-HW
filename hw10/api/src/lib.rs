//! Контракт HTTP API: DTO и пути, общие для сервера и wasm-клиента.

use std::fmt;

use serde::{Deserialize, Serialize};

// Команды: POST, ответ — `CommandAcceptedDto`. Создание идёт POST-запросом на адрес списка:
// `TENANTS` с `CreateTenantRequest`, `USERS` с `CreateUserRequest`, `TICKETS` с `CreateTicketRequest`.

pub fn open_ticket(ticket_id: usize) -> String {
    format!("/api/tickets/{ticket_id}/open")
}

pub fn close_ticket(ticket_id: usize) -> String {
    format!("/api/tickets/{ticket_id}/close")
}

pub fn assign_agent(ticket_id: usize) -> String {
    format!("/api/tickets/{ticket_id}/assign")
}

// Запросы: GET к read-модели.

/// Все тикеты; поддерживает фильтры `?status=Open|InProgress|Closed` и `?tenant=<id>`.
pub const TICKETS: &str = "/api/tickets";
pub const USERS: &str = "/api/users";
pub const TENANTS: &str = "/api/tenants";

pub fn ticket(ticket_id: usize) -> String {
    format!("/api/tickets/{ticket_id}")
}

pub fn tenant(tenant_id: usize) -> String {
    format!("/api/tenants/{tenant_id}")
}

/// Входящие уведомления пользователя, от старых к новым.
pub fn notifications_of(user_id: usize) -> String {
    format!("/api/users/{user_id}/notifications")
}

/// Незакрытые тикеты, назначенные на агента.
pub fn active_tickets_of_agent(agent_id: usize) -> String {
    format!("/api/agents/{agent_id}/tickets")
}

/// Открытые тикеты тенанта, которые ещё никто не взял в работу.
pub fn unassigned_tickets_of_tenant(tenant_id: usize) -> String {
    format!("/api/tenants/{tenant_id}/tickets/unassigned")
}

/// Тикеты, закрытые за последние `days` дней.
pub fn tickets_closed_within(days: u64) -> String {
    format!("/api/tickets/closed?days={days}")
}

/// Long polling: изменения после позиции `after`. Если их ещё нет, сервер держит запрос,
/// пока они не появятся или не истечёт таймаут, и тогда отвечает пустым списком.
/// Без позиции (первый запрос) — всё состояние сразу.
pub fn changes(after: Option<Cursor>) -> String {
    match after {
        None => "/api/changes".to_owned(),
        Some(Cursor { epoch, version }) => format!("/api/changes?epoch={epoch}&since={version}"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TicketStatus {
    Open,
    InProgress,
    Closed,
}

impl fmt::Display for TicketStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Open => "Open",
            Self::InProgress => "In progress",
            Self::Closed => "Closed",
        };
        f.write_str(name)
    }
}

/// Уровень обслуживания клиента в CRM: от него зависит SLA.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CustomerTier {
    Gold,
    Silver,
    #[default]
    Bronze,
}

impl fmt::Display for CustomerTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Gold => "Gold",
            Self::Silver => "Silver",
            Self::Bronze => "Bronze",
        };
        f.write_str(name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Customer,
    Agent,
    Admin,
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Customer => "Customer",
            Self::Agent => "Agent",
            Self::Admin => "Admin",
        };
        f.write_str(name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloseReason {
    /// Проблема решена.
    Resolved,
    /// Истёк срок неактивности.
    Inactivity,
}

impl fmt::Display for CloseReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Resolved => "resolved",
            Self::Inactivity => "inactivity",
        };
        f.write_str(name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReminderKind {
    /// Тикет простаивает.
    Stale,
    /// Скоро тикет закроется сам.
    ClosingSoon,
}

impl fmt::Display for ReminderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Stale => "stale",
            Self::ClosingSoon => "closing soon",
        };
        f.write_str(name)
    }
}

/// Кому адресовано уведомление.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Recipient {
    Agent(usize),
    Admins,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationKind {
    NewTicket,
    Reopened,
    Assigned,
    Unassigned,
    Reminder(ReminderKind),
    Closed(CloseReason),
}

impl fmt::Display for NotificationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NewTicket => f.write_str("new ticket needs an agent"),
            Self::Reopened => f.write_str("reopened and needs an agent"),
            Self::Assigned => f.write_str("assigned to you"),
            Self::Unassigned => f.write_str("reassigned to another agent"),
            Self::Reminder(kind) => write!(f, "reminder: {kind}"),
            Self::Closed(reason) => write!(f, "closed: {reason}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationDto {
    pub notification_id: usize,
    pub recipient: Recipient,
    pub ticket_id: usize,
    pub kind: NotificationKind,
    /// Время в секундах Unix-времени.
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketDto {
    pub ticket_id: usize,
    pub category: String,
    pub description: String,
    pub status: TicketStatus,
    pub assigned_agent: Option<usize>,
    pub tenant_id: usize,
    /// Время создания в секундах Unix-времени.
    pub created_at: u64,
    /// Время закрытия в секундах Unix-времени; `None`, пока тикет открыт.
    pub closed_at: Option<u64>,
    /// Почему тикет закрыли; `None`, пока он открыт.
    pub close_reason: Option<CloseReason>,
    /// Последняя активность в секундах Unix-времени: от неё считаются сроки.
    pub last_activity: u64,
    /// Когда тикет закроется сам, если активности не будет; `None`, если он закрыт.
    pub closes_at: Option<u64>,
    /// Сколько напоминаний отправлено с момента последней активности.
    pub reminders_sent: u32,
    /// Уровень клиента в CRM на момент создания.
    pub tier: CustomerTier,
    /// К какому сроку клиенту обещано решение (SLA), в секундах Unix-времени.
    pub sla_due_at: u64,
    /// Уложились ли в SLA; `None`, пока тикет открыт.
    pub sla_met: Option<bool>,
    /// Ответ ERP о ресурсах; `None`, пока ресурсы не проверены.
    pub resources: Option<ResourceAvailability>,
    /// Имя партнёра, которому передан тикет.
    pub partner: Option<String>,
    /// Когда тикет передан партнёру, в секундах Unix-времени.
    pub handed_over_at: Option<u64>,
    /// Отчёты партнёра от старых к новым.
    pub partner_updates: Vec<PartnerUpdateDto>,
}

/// Хватает ли своих ресурсов на решение тикета — ответ ERP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceAvailability {
    Available,
    Unavailable,
}

impl fmt::Display for ResourceAvailability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Available => "in stock",
            Self::Unavailable => "out of stock",
        };
        f.write_str(name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartnerUpdateDto {
    pub comment: String,
    /// Время в секундах Unix-времени.
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserDto {
    pub user_id: usize,
    pub user_name: String,
    pub role: Role,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantDto {
    pub tenant_id: usize,
    pub tenant_name: String,
    pub discount_percent: u8,
    pub open_tickets: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateTenantRequest {
    pub tenant_name: String,
    /// С каким уровнем завести клиента в CRM; если не указан — Bronze.
    #[serde(default)]
    pub tier: CustomerTier,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateUserRequest {
    pub user_name: String,
    pub role: Role,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateTicketRequest {
    pub category: String,
    pub description: String,
    pub tenant_id: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignAgentRequest {
    pub agent_id: usize,
}

/// Команда выполнена; её результат виден в read-модели, начиная с версии `version`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandAcceptedDto {
    pub version: u64,
}

/// Позиция клиента в потоке изменений сервера.
///
/// `epoch` — идентификатор запуска сервера: версии разных запусков несравнимы. Если сервер
/// видит чужую эпоху, он отвечает всем состоянием заново, и клиенту стоит выбросить своё.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub epoch: u64,
    pub version: u64,
}

/// Ответ long polling: записи, изменённые после запрошенной позиции, в последнем состоянии.
/// Следующий запрос — с позицией [`ChangesDto::cursor`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangesDto {
    pub epoch: u64,
    pub version: u64,
    /// `true` — ответ содержит всё состояние, а не изменения: клиент новый или сервер
    /// перезапускался.
    pub full: bool,
    pub tickets: Vec<TicketDto>,
    pub tenants: Vec<TenantDto>,
    pub users: Vec<UserDto>,
    /// Новые уведомления всех получателей, от старых к новым; каждое приходит один раз.
    pub notifications: Vec<NotificationDto>,
}

impl ChangesDto {
    pub fn cursor(&self) -> Cursor {
        Cursor {
            epoch: self.epoch,
            version: self.version,
        }
    }
}

/// Тело любого ответа с ошибкой.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorDto {
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_kinds_read_as_text() {
        let text = |kind: NotificationKind| kind.to_string();
        assert_eq!(text(NotificationKind::Assigned), "assigned to you");
        assert_eq!(
            text(NotificationKind::Reminder(ReminderKind::ClosingSoon)),
            "reminder: closing soon"
        );
        assert_eq!(
            text(NotificationKind::Closed(CloseReason::Inactivity)),
            "closed: inactivity"
        );
    }
}
