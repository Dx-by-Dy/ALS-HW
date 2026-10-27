//! Контракт HTTP API: DTO и пути, общие для сервера и wasm-клиента.

use std::fmt;

use serde::{Deserialize, Serialize};

// Команды: POST, ответ — `CommandAcceptedDto`.

pub fn apply_discount(tenant_id: usize) -> String {
    format!("/api/tenants/{tenant_id}/discount")
}

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
