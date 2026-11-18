use std::error::Error;
use std::fmt;

use super::{ReminderKind, TicketId};
use crate::domain::{
    partner::PartnerId,
    user::{Role, UserId},
};

/// Нарушения инвариантов агрегата `Ticket`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TicketError {
    /// Тикет с таким идентификатором уже создан.
    AlreadyExists(TicketId),
    /// Команда пришла в пустой стрим: тикета нет.
    NotFound,
    /// Описание проблемы не может быть пустым.
    EmptyDescription,
    /// Категория не может быть пустой.
    EmptyCategory,
    /// Назначить на тикет можно только пользователя с ролью `Agent`.
    NotAnAgent { user_id: UserId, role: Role },
    /// Этот агент уже назначен на тикет.
    AlreadyAssigned(UserId),
    /// Закрытый тикет больше не меняется.
    Closed(TicketId),
    /// Открыть заново можно только закрытый тикет.
    AlreadyOpen(TicketId),
    /// Закрыть по неактивности можно только тикет, простоявший дольше таймаута.
    StillActive { idle_secs: u64 },
    /// Тикет простаивает недостаточно долго для такого напоминания.
    ReminderNotDue { kind: ReminderKind, idle_secs: u64 },
    /// С последней активности уже отправлено это или более позднее напоминание.
    ReminderAlreadySent(ReminderKind),
    /// Ресурсы уже проверены с момента создания или повторного открытия.
    ResourcesAlreadyChecked(TicketId),
    /// Партнёр нужен, только если проверка показала, что своих ресурсов нет.
    PartnerNotNeeded(TicketId),
    /// Тикет уже передан партнёру.
    AlreadyWithPartner(PartnerId),
    /// Этот партнёр тикетом не занимается.
    NotWithPartner(PartnerId),
    /// Отчёт партнёра не может быть пустым.
    EmptyComment,
}

impl fmt::Display for TicketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyExists(id) => write!(f, "{id} already exists"),
            Self::NotFound => write!(f, "ticket does not exist"),
            Self::EmptyDescription => write!(f, "ticket description must not be empty"),
            Self::EmptyCategory => write!(f, "ticket category must not be empty"),
            Self::NotAnAgent { user_id, role } => {
                write!(
                    f,
                    "{user_id} has role {role} and cannot be assigned to a ticket"
                )
            }
            Self::AlreadyAssigned(id) => write!(f, "{id} is already assigned to the ticket"),
            Self::Closed(id) => write!(f, "{id} is closed"),
            Self::AlreadyOpen(id) => write!(f, "{id} is already open"),
            Self::StillActive { idle_secs } => {
                write!(
                    f,
                    "ticket was active {idle_secs}s ago and cannot be closed for inactivity"
                )
            }
            Self::ReminderNotDue { kind, idle_secs } => {
                write!(
                    f,
                    "ticket was active {idle_secs}s ago, the {kind} reminder is not due yet"
                )
            }
            Self::ReminderAlreadySent(kind) => {
                write!(
                    f,
                    "the {kind} reminder is no longer needed: it or a later one was sent"
                )
            }
            Self::ResourcesAlreadyChecked(id) => {
                write!(f, "resources for {id} are already checked")
            }
            Self::PartnerNotNeeded(id) => write!(
                f,
                "{id} needs no partner: resources were not reported unavailable"
            ),
            Self::AlreadyWithPartner(partner) => {
                write!(f, "ticket is already handed to {partner}")
            }
            Self::NotWithPartner(partner) => write!(f, "ticket is not handed to {partner}"),
            Self::EmptyComment => write!(f, "progress comment must not be empty"),
        }
    }
}

impl Error for TicketError {}
