//! Порты к внешним системам — CRM и ERP.
//!
//! Порты говорят на языке системы тикетов: тенант, уровень клиента, статус тикета, категория.
//! Как эти понятия выглядят у CRM и ERP, знают только адаптеры Anti-Corruption Layer
//! в крейте `integration`, которые эти порты реализуют.

use std::error::Error;
use std::fmt;
use std::future::Future;

use crate::domain::{
    sla::CustomerTier,
    tenant::TenantId,
    ticket::{CloseReason, ResourceAvailability, TicketId},
    time::Timestamp,
};

/// Что система тикетов знает о клиенте из CRM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerProfile {
    pub tenant_id: TenantId,
    pub name: String,
    /// Куда CRM пишет клиенту; `None`, если адреса нет.
    pub email: Option<String>,
    pub tier: CustomerTier,
    /// Может ли клиент заводить тикеты: договор действует, обслуживание не приостановлено.
    pub active: bool,
}

/// Что сообщить клиенту о его тикете.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CustomerNotice {
    pub tenant_id: TenantId,
    pub ticket_id: TicketId,
    pub kind: CustomerNoticeKind,
    pub at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomerNoticeKind {
    /// Обращение принято; решить его обещаем к `due_at`.
    Received { due_at: Timestamp },
    /// Работа по обращению завершена.
    Closed { reason: CloseReason, sla_met: bool },
    /// Обращение открыто заново; новый срок — `due_at`.
    Reopened { due_at: Timestamp },
}

/// Сбой внешней системы, описанный без её терминов.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalError {
    /// Система не ответила; позже запрос можно повторить.
    Unavailable {
        system: &'static str,
        reason: String,
    },
    /// Система ответила, но ответ нельзя перевести в модель тикетов или она отвергла запрос.
    Rejected {
        system: &'static str,
        reason: String,
    },
}

impl fmt::Display for ExternalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { system, reason } => write!(f, "{system} is unavailable: {reason}"),
            Self::Rejected { system, reason } => {
                write!(f, "{system} rejected the request: {reason}")
            }
        }
    }
}

impl Error for ExternalError {}

/// Справочник клиентов: CRM.
pub trait CustomerDirectory {
    /// Клиент, стоящий за тенантом; `None`, если CRM о нём не знает.
    fn find_customer(
        &self,
        tenant_id: TenantId,
    ) -> impl Future<Output = Result<Option<CustomerProfile>, ExternalError>> + Send;
}

/// Канал сообщений клиенту: CRM пишет ему сама, по своим настройкам связи.
pub trait CustomerNotifier {
    fn notify_customer(
        &self,
        notice: &CustomerNotice,
    ) -> impl Future<Output = Result<(), ExternalError>> + Send;
}

/// Склад и ресурсы: ERP.
pub trait Inventory {
    /// Хватает ли своих ресурсов, чтобы решить тикет этой категории.
    fn check_resources(
        &self,
        category: &str,
    ) -> impl Future<Output = Result<ResourceAvailability, ExternalError>> + Send;
}
