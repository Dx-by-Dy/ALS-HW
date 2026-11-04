//! Сервис связи с бэкендом. Команды отправляются на командную сторону и возвращают
//! только версию, состояние приходит через long polling (`changes`), а запросы
//! ([`TicketQuery`]) читают read-модель сервера.

use std::error::Error;
use std::fmt;

use api::{
    AssignAgentRequest, ChangesDto, CommandAcceptedDto, Cursor, ErrorDto, TicketDto, UserDto,
};
use serde::de::DeserializeOwned;

use super::http::{HttpClient, HttpRequest, TransportError};
use super::{TicketRules, ValidationError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// Команда отклонена ещё на клиенте, запрос не отправлялся.
    Validation(ValidationError),
    /// Сервер ответил кодом ошибки.
    Server {
        status: u16,
        message: String,
    },
    Transport(TransportError),
    /// Сервер ответил успехом, но тело не разобрать.
    InvalidResponse(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(e) => e.fmt(f),
            Self::Server { status, message } => {
                write!(f, "server rejected the request ({status}): {message}")
            }
            Self::Transport(e) => write!(f, "cannot reach the server: {e}"),
            Self::InvalidResponse(e) => write!(f, "unexpected server response: {e}"),
        }
    }
}

impl Error for ApiError {}

impl From<ValidationError> for ApiError {
    fn from(e: ValidationError) -> Self {
        Self::Validation(e)
    }
}

/// Запрос к read-модели сервера.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TicketQuery {
    /// Незакрытые тикеты, назначенные на агента.
    ActiveOfAgent(usize),
    /// Открытые тикеты тенанта, которые ещё никто не взял в работу.
    UnassignedOfTenant(usize),
    /// Тикеты, закрытые за последние N дней.
    ClosedWithin(u64),
}

impl TicketQuery {
    pub fn path(self) -> String {
        match self {
            Self::ActiveOfAgent(agent_id) => api::active_tickets_of_agent(agent_id),
            Self::UnassignedOfTenant(tenant_id) => api::unassigned_tickets_of_tenant(tenant_id),
            Self::ClosedWithin(days) => api::tickets_closed_within(days),
        }
    }
}

pub struct TicketApiService<H> {
    http: H,
}

impl<H: HttpClient> TicketApiService<H> {
    pub fn new(http: H) -> Self {
        Self { http }
    }

    /// Long polling: изменения после позиции `after`; без позиции — всё состояние сразу.
    /// Если изменений нет, сервер держит запрос до своего таймаута.
    pub async fn changes(&self, after: Option<Cursor>) -> Result<ChangesDto, ApiError> {
        self.request(HttpRequest::get(api::changes(after))).await
    }

    pub async fn query(&self, query: TicketQuery) -> Result<Vec<TicketDto>, ApiError> {
        self.request(HttpRequest::get(query.path())).await
    }

    /// Открывает закрытый тикет. Ответ — версия, с которой результат виден в `changes`.
    pub async fn open_ticket(&self, ticket: &TicketDto) -> Result<CommandAcceptedDto, ApiError> {
        ticket.validate_open()?;
        let path = api::open_ticket(ticket.ticket_id);
        self.request(HttpRequest::post(path, None)).await
    }

    /// Закрывает открытый тикет. Ответ — версия, с которой результат виден в `changes`.
    pub async fn close_ticket(&self, ticket: &TicketDto) -> Result<CommandAcceptedDto, ApiError> {
        ticket.validate_close()?;
        let path = api::close_ticket(ticket.ticket_id);
        self.request(HttpRequest::post(path, None)).await
    }

    pub async fn assign_agent(
        &self,
        ticket: &TicketDto,
        agent: &UserDto,
    ) -> Result<CommandAcceptedDto, ApiError> {
        ticket.validate_assign(agent)?;
        let body = AssignAgentRequest {
            agent_id: agent.user_id,
        };
        let body = serde_json::to_string(&body).expect("request serializes to JSON");
        let path = api::assign_agent(ticket.ticket_id);
        self.request(HttpRequest::post(path, Some(body))).await
    }

    async fn request<T: DeserializeOwned>(&self, request: HttpRequest) -> Result<T, ApiError> {
        let response = self.http.send(request).await.map_err(ApiError::Transport)?;
        if !response.is_success() {
            let message = serde_json::from_str::<ErrorDto>(&response.body)
                .map_or(response.body, |dto| dto.error);
            return Err(ApiError::Server {
                status: response.status,
                message,
            });
        }
        serde_json::from_str(&response.body).map_err(|e| ApiError::InvalidResponse(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;

    use super::*;
    use crate::model::fixtures::{changes, ticket, user};
    use crate::model::http::Method;
    use crate::model::mock_http::MockHttpClient;
    use api::{Role, TicketStatus};

    fn service() -> (TicketApiService<MockHttpClient>, MockHttpClient) {
        let http = MockHttpClient::default();
        (TicketApiService::new(http.clone()), http)
    }

    fn accepted(version: u64) -> CommandAcceptedDto {
        CommandAcceptedDto { version }
    }

    #[test]
    fn close_success_returns_version() {
        let (service, http) = service();
        http.respond_json(200, &accepted(42));

        let open = ticket(2, TicketStatus::Open);
        assert_eq!(block_on(service.close_ticket(&open)), Ok(accepted(42)));
        assert_eq!(
            http.requests(),
            [HttpRequest::post("/api/tickets/2/close", None)]
        );
    }

    #[test]
    fn close_failure_is_an_error() {
        let (service, http) = service();
        http.respond_error(409, "ticket-2 is closed");

        let result = block_on(service.close_ticket(&ticket(2, TicketStatus::Open)));

        assert_eq!(
            result,
            Err(ApiError::Server {
                status: 409,
                message: "ticket-2 is closed".to_owned()
            })
        );
    }

    #[test]
    fn open_success_returns_version() {
        let (service, http) = service();
        http.respond_json(200, &accepted(7));

        let closed = ticket(7, TicketStatus::Closed);
        assert_eq!(block_on(service.open_ticket(&closed)), Ok(accepted(7)));
        assert_eq!(
            http.requests(),
            [HttpRequest::post("/api/tickets/7/open", None)]
        );
    }

    #[test]
    fn open_failure_is_an_error() {
        let (service, http) = service();
        http.respond_error(500, "storage error");

        let result = block_on(service.open_ticket(&ticket(7, TicketStatus::Closed)));

        assert!(matches!(result, Err(ApiError::Server { status: 500, .. })));
    }

    #[test]
    fn invalid_command_is_not_sent() {
        let (service, http) = service();
        assert_eq!(
            block_on(service.close_ticket(&ticket(2, TicketStatus::Closed))),
            Err(ApiError::Validation(ValidationError::AlreadyClosed(2)))
        );
        assert_eq!(
            block_on(service.open_ticket(&ticket(2, TicketStatus::Open))),
            Err(ApiError::Validation(ValidationError::AlreadyOpen(2)))
        );
        assert!(http.requests().is_empty());
    }

    #[test]
    fn transport_failure_is_reported() {
        let (service, http) = service();
        http.fail("connection refused");
        assert_eq!(
            block_on(service.close_ticket(&ticket(2, TicketStatus::Open))),
            Err(ApiError::Transport(TransportError(
                "connection refused".to_owned()
            )))
        );
    }

    #[test]
    fn error_without_json_body_keeps_raw_text() {
        let (service, http) = service();
        http.respond_raw(502, "Bad Gateway");
        assert_eq!(
            block_on(service.close_ticket(&ticket(2, TicketStatus::Open))),
            Err(ApiError::Server {
                status: 502,
                message: "Bad Gateway".to_owned()
            })
        );
    }

    #[test]
    fn malformed_success_body_is_an_error() {
        let (service, http) = service();
        http.respond_raw(200, "{}");
        assert!(matches!(
            block_on(service.close_ticket(&ticket(2, TicketStatus::Open))),
            Err(ApiError::InvalidResponse(_))
        ));
    }

    #[test]
    fn assign_sends_agent_id() {
        let (service, http) = service();
        http.respond_json(200, &accepted(3));

        let alice = user(5, "Alice", Role::Agent);
        let result = block_on(service.assign_agent(&ticket(2, TicketStatus::Open), &alice));

        assert_eq!(result, Ok(accepted(3)));
        let request = &http.requests()[0];
        assert_eq!(request.method, Method::Post);
        assert_eq!(request.path, "/api/tickets/2/assign");
        assert_eq!(request.body.as_deref(), Some(r#"{"agent_id":5}"#));
    }

    #[test]
    fn assigning_non_agent_is_not_sent() {
        let (service, http) = service();
        let bob = user(3, "Bob", Role::Customer);
        assert!(matches!(
            block_on(service.assign_agent(&ticket(2, TicketStatus::Open), &bob)),
            Err(ApiError::Validation(ValidationError::NotAnAgent { .. }))
        ));
        assert!(http.requests().is_empty());
    }

    #[test]
    fn changes_are_asked_after_cursor() {
        let (service, http) = service();
        let all = changes(9, 20, true, vec![ticket(1, TicketStatus::Open)]);
        http.respond_json(200, &all)
            .respond_json(200, &changes(9, 21, false, vec![]));

        assert_eq!(block_on(service.changes(None)), Ok(all.clone()));
        block_on(service.changes(Some(all.cursor()))).unwrap();
        let paths: Vec<_> = http.requests().into_iter().map(|r| r.path).collect();
        assert_eq!(paths, ["/api/changes", "/api/changes?epoch=9&since=20"]);
    }

    #[test]
    fn queries_go_to_read_side_paths() {
        let (service, http) = service();
        for _ in 0..3 {
            http.respond_json(200, &[ticket(4, TicketStatus::Closed)]);
        }
        for query in [
            TicketQuery::ActiveOfAgent(1),
            TicketQuery::UnassignedOfTenant(2),
            TicketQuery::ClosedWithin(7),
        ] {
            assert_eq!(block_on(service.query(query)).unwrap().len(), 1);
        }
        let paths: Vec<_> = http.requests().into_iter().map(|r| r.path).collect();
        assert_eq!(
            paths,
            [
                "/api/agents/1/tickets",
                "/api/tenants/2/tickets/unassigned",
                "/api/tickets/closed?days=7",
            ]
        );
    }
}
