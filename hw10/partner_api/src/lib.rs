//! Open-Host Service для партнёров: опубликованный язык (Published Language), версия 1.
//!
//! Это публичный контракт, по которому партнёры следят за переданными им тикетами и сообщают
//! о ходе работы. Он отделён от внутреннего `api`, на котором говорит наш веб-клиент:
//! внутренний API меняется вместе с клиентом, а этот — только новой версией (`/partner/v2`),
//! чтобы не ломать интеграции партнёров.
//!
//! Правила контракта:
//! - каждый запрос несёт заголовок `Authorization: Bearer <токен партнёра>`, без него — 401;
//! - партнёр видит только тикеты, переданные ему; чужой тикет для него не существует — 404;
//! - время — секунды от эпохи Unix, перечисления — строки в `snake_case`;
//! - ошибка — [`PartnerError`] с машинным кодом и текстом для людей.

use serde::{Deserialize, Serialize};

/// Корень версии 1.
pub const BASE: &str = "/partner/v1";

/// `GET` — тикеты, переданные партнёру, по возрастанию номера.
pub const TICKETS: &str = "/partner/v1/tickets";

/// `GET` — один тикет.
pub fn ticket(ticket_id: usize) -> String {
    format!("{TICKETS}/{ticket_id}")
}

/// `POST` с [`TicketUpdateRequest`] — сообщить о ходе работы; ответ — обновлённый [`PartnerTicket`].
pub fn ticket_updates(ticket_id: usize) -> String {
    format!("{TICKETS}/{ticket_id}/updates")
}

/// Значение заголовка `Authorization` для токена партнёра.
pub fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// Состояние тикета глазами партнёра.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartnerTicketStatus {
    /// Тикет у партнёра в работе.
    Open,
    /// Проблема решена — партнёром или нашими агентами.
    Resolved,
    /// Тикет закрыт, потому что по нему долго не было активности.
    ClosedForInactivity,
}

/// Тикет, переданный партнёру.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartnerTicket {
    pub ticket_id: usize,
    pub category: String,
    pub description: String,
    /// Имя клиента.
    pub customer: String,
    pub status: PartnerTicketStatus,
    /// Когда тикет передали партнёру.
    pub handed_over_at: u64,
    /// К какому сроку клиенту обещано решение (SLA).
    pub due_at: u64,
    /// Когда тикет закрыт; `null`, пока он открыт.
    pub closed_at: Option<u64>,
    /// Отчёты партнёра от старых к новым.
    pub updates: Vec<PartnerUpdate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartnerUpdate {
    pub comment: String,
    pub at: u64,
}

/// Что партнёр сообщает о тикете.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateStatus {
    /// Работа идёт; комментарий обязателен.
    InProgress,
    /// Проблема решена, тикет закрывается; комментарий необязателен.
    Resolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketUpdateRequest {
    pub status: UpdateStatus,
    #[serde(default)]
    pub comment: String,
}

/// Машинный код ошибки.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Нет токена или он неизвестен — 401.
    Unauthorized,
    /// Тикета нет или он передан не этому партнёру — 404.
    NotFound,
    /// Тикет уже закрыт — 409.
    Closed,
    /// В запросе неверные данные, например пустой комментарий — 422.
    Invalid,
    /// Сервис временно не может принять запрос — 503.
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartnerError {
    pub code: ErrorCode,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn paths_are_versioned() {
        assert_eq!(TICKETS, "/partner/v1/tickets");
        assert_eq!(ticket(4), "/partner/v1/tickets/4");
        assert_eq!(ticket_updates(4), "/partner/v1/tickets/4/updates");
        assert_eq!(bearer("abc"), "Bearer abc");
    }

    #[test]
    fn ticket_has_the_published_shape() {
        let ticket = PartnerTicket {
            ticket_id: 4,
            category: "Hardware".to_owned(),
            description: "Printer is offline".to_owned(),
            customer: "Acme".to_owned(),
            status: PartnerTicketStatus::ClosedForInactivity,
            handed_over_at: 100,
            due_at: 86_500,
            closed_at: None,
            updates: vec![PartnerUpdate {
                comment: "On site".to_owned(),
                at: 200,
            }],
        };
        assert_eq!(
            serde_json::to_value(&ticket).unwrap(),
            json!({
                "ticket_id": 4,
                "category": "Hardware",
                "description": "Printer is offline",
                "customer": "Acme",
                "status": "closed_for_inactivity",
                "handed_over_at": 100,
                "due_at": 86_500,
                "closed_at": null,
                "updates": [{"comment": "On site", "at": 200}],
            })
        );
    }

    #[test]
    fn update_comment_may_be_omitted() {
        let request: TicketUpdateRequest =
            serde_json::from_value(json!({"status": "resolved"})).unwrap();
        assert_eq!(
            request,
            TicketUpdateRequest {
                status: UpdateStatus::Resolved,
                comment: String::new(),
            }
        );
        let unknown = serde_json::from_value::<TicketUpdateRequest>(json!({"status": "done"}));
        assert!(unknown.is_err());
    }

    #[test]
    fn error_carries_machine_code() {
        let error = PartnerError {
            code: ErrorCode::NotFound,
            message: "ticket-5 is not handed to you".to_owned(),
        };
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            json!({"code": "not_found", "message": "ticket-5 is not handed to you"})
        );
    }
}
