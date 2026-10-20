//! Контракт HTTP API: DTO и пути, общие для сервера и wasm-клиента.

use std::fmt;

use serde::{Deserialize, Serialize};

pub const TICKETS: &str = "/api/tickets";
pub const USERS: &str = "/api/users";
pub const TENANTS: &str = "/api/tenants";

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

/// Тело любого ответа с ошибкой.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorDto {
    pub error: String,
}
