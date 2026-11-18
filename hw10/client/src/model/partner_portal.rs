//! Портал партнёра: клиент Open-Host Service `/partner/v1`.
//!
//! Говорит на опубликованном языке `partner_api`, а не на внутреннем `api`: так панель
//! партнёра работает ровно тем же контрактом, что и интеграция настоящего партнёра.

use partner_api::{PartnerError, PartnerTicket, TicketUpdateRequest, UpdateStatus};
use serde::de::DeserializeOwned;

use super::http::{HttpClient, HttpRequest};
use super::required;
use super::ticket_api::ApiError;

pub struct PartnerPortalService<H> {
    http: H,
}

impl<H: HttpClient> PartnerPortalService<H> {
    pub fn new(http: H) -> Self {
        Self { http }
    }

    /// Тикеты, переданные партнёру с токеном `token`.
    pub async fn tickets(&self, token: &str) -> Result<Vec<PartnerTicket>, ApiError> {
        let request = HttpRequest::get(partner_api::TICKETS);
        self.request(token, request).await
    }

    /// Сообщает о ходе работы или о решении. Без комментария можно только закрыть тикет.
    pub async fn update(
        &self,
        token: &str,
        ticket_id: usize,
        status: UpdateStatus,
        comment: &str,
    ) -> Result<PartnerTicket, ApiError> {
        let comment = match status {
            UpdateStatus::InProgress => required("comment", comment)?,
            UpdateStatus::Resolved => comment.trim().to_owned(),
        };
        let body = TicketUpdateRequest { status, comment };
        let body = serde_json::to_string(&body).expect("request serializes to JSON");
        let request = HttpRequest::post(partner_api::ticket_updates(ticket_id), Some(body));
        self.request(token, request).await
    }

    async fn request<T: DeserializeOwned>(
        &self,
        token: &str,
        request: HttpRequest,
    ) -> Result<T, ApiError> {
        let request = request.with_authorization(partner_api::bearer(token));
        let response = self.http.send(request).await.map_err(ApiError::Transport)?;
        if !response.is_success() {
            let message = serde_json::from_str::<PartnerError>(&response.body)
                .map_or(response.body, |e| e.message);
            return Err(ApiError::Server {
                status: response.status,
                message,
            });
        }
        serde_json::from_str(&response.body).map_err(|e| ApiError::InvalidResponse(e.to_string()))
    }
}

#[cfg(test)]
pub mod fixtures {
    use partner_api::{PartnerTicket, PartnerTicketStatus};

    pub fn partner_ticket(id: usize, status: PartnerTicketStatus) -> PartnerTicket {
        PartnerTicket {
            ticket_id: id,
            category: "Hardware".to_owned(),
            description: "Printer is offline".to_owned(),
            customer: "Acme".to_owned(),
            status,
            handed_over_at: 0,
            due_at: 86_400,
            closed_at: None,
            updates: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use partner_api::{ErrorCode, PartnerTicketStatus};

    use super::fixtures::partner_ticket;
    use super::*;
    use crate::model::ValidationError;
    use crate::model::mock_http::MockHttpClient;

    fn portal() -> (PartnerPortalService<MockHttpClient>, MockHttpClient) {
        let http = MockHttpClient::default();
        (PartnerPortalService::new(http.clone()), http)
    }

    #[test]
    fn tickets_are_asked_with_partner_token() {
        let (portal, http) = portal();
        http.respond_json(200, &vec![partner_ticket(4, PartnerTicketStatus::Open)]);
        let tickets = block_on(portal.tickets("fixit-token")).unwrap();
        assert_eq!(tickets[0].ticket_id, 4);
        assert_eq!(
            http.requests(),
            [HttpRequest::get("/partner/v1/tickets").with_authorization("Bearer fixit-token")]
        );
    }

    #[test]
    fn update_posts_published_request() {
        let (portal, http) = portal();
        http.respond_json(200, &partner_ticket(4, PartnerTicketStatus::Resolved));
        let ticket = block_on(portal.update("t", 4, UpdateStatus::Resolved, "  Done ")).unwrap();
        assert_eq!(ticket.status, PartnerTicketStatus::Resolved);
        assert_eq!(
            http.requests(),
            [HttpRequest::post(
                "/partner/v1/tickets/4/updates",
                Some(r#"{"status":"resolved","comment":"Done"}"#.to_owned())
            )
            .with_authorization("Bearer t")]
        );
    }

    #[test]
    fn progress_without_comment_is_not_sent() {
        let (portal, http) = portal();
        assert_eq!(
            block_on(portal.update("t", 4, UpdateStatus::InProgress, " ")),
            Err(ApiError::Validation(ValidationError::Blank("comment")))
        );
        assert_eq!(http.requests(), []);
    }

    #[test]
    fn partner_error_message_is_shown() {
        let (portal, http) = portal();
        let error = PartnerError {
            code: ErrorCode::Unauthorized,
            message: "a valid partner token is required".to_owned(),
        };
        http.respond_json(401, &error);
        assert_eq!(
            block_on(portal.tickets("wrong")),
            Err(ApiError::Server {
                status: 401,
                message: "a valid partner token is required".to_owned(),
            })
        );
    }
}
