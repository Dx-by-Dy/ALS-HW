//! Сервис связи с бэкендом: отправляет команды на сервер и возвращает состояние из его ответа.

use std::error::Error;
use std::fmt;

use api::{AssignAgentRequest, ErrorDto, TenantDto, TicketDto, UserDto};
use serde::de::DeserializeOwned;

use super::http::{HttpClient, HttpRequest, TransportError};
use super::{Tenant, TenantId, Ticket, User, ValidationError};

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

pub struct TicketApiService<H> {
    http: H,
}

impl<H: HttpClient> TicketApiService<H> {
    pub fn new(http: H) -> Self {
        Self { http }
    }

    pub async fn tickets(&self) -> Result<Vec<Ticket>, ApiError> {
        let dtos: Vec<TicketDto> = self.request(HttpRequest::get(api::TICKETS)).await?;
        Ok(dtos.into_iter().map(Ticket::from).collect())
    }

    pub async fn users(&self) -> Result<Vec<User>, ApiError> {
        let dtos: Vec<UserDto> = self.request(HttpRequest::get(api::USERS)).await?;
        Ok(dtos.into_iter().map(User::from).collect())
    }

    pub async fn tenants(&self) -> Result<Vec<Tenant>, ApiError> {
        let dtos: Vec<TenantDto> = self.request(HttpRequest::get(api::TENANTS)).await?;
        Ok(dtos.into_iter().map(Tenant::from).collect())
    }

    /// Открывает закрытый тикет и возвращает его состояние после команды.
    pub async fn open_ticket(&self, ticket: &Ticket) -> Result<Ticket, ApiError> {
        ticket.validate_open()?;
        let path = api::open_ticket(ticket.ticket_id.0);
        self.ticket_command(HttpRequest::post(path, None)).await
    }

    /// Закрывает открытый тикет и возвращает его состояние после команды.
    pub async fn close_ticket(&self, ticket: &Ticket) -> Result<Ticket, ApiError> {
        ticket.validate_close()?;
        let path = api::close_ticket(ticket.ticket_id.0);
        self.ticket_command(HttpRequest::post(path, None)).await
    }

    pub async fn assign_agent(&self, ticket: &Ticket, agent: &User) -> Result<Ticket, ApiError> {
        ticket.validate_assign(agent)?;
        let body = AssignAgentRequest {
            agent_id: agent.user_id.0,
        };
        let body = serde_json::to_string(&body).expect("request serializes to JSON");
        let path = api::assign_agent(ticket.ticket_id.0);
        self.ticket_command(HttpRequest::post(path, Some(body)))
            .await
    }

    /// Пересчитывает скидку тенанта по числу его открытых тикетов.
    pub async fn apply_discount(&self, tenant_id: TenantId) -> Result<Tenant, ApiError> {
        let path = api::apply_discount(tenant_id.0);
        let dto: TenantDto = self.request(HttpRequest::post(path, None)).await?;
        Ok(dto.into())
    }

    async fn ticket_command(&self, request: HttpRequest) -> Result<Ticket, ApiError> {
        let dto: TicketDto = self.request(request).await?;
        Ok(dto.into())
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
    use crate::model::fixtures::{ticket, user};
    use crate::model::http::Method;
    use crate::model::mock_http::MockHttpClient;
    use crate::model::{Role, TicketId, TicketStatus, UserId};

    fn service() -> (TicketApiService<MockHttpClient>, MockHttpClient) {
        let http = MockHttpClient::default();
        (TicketApiService::new(http.clone()), http)
    }

    fn dto(id: usize, status: TicketStatus, agent: Option<usize>) -> TicketDto {
        TicketDto {
            ticket_id: id,
            category: "Technical Issue".to_owned(),
            description: "VPN is down".to_owned(),
            status,
            assigned_agent: agent,
            tenant_id: 1,
        }
    }

    #[test]
    fn close_success_returns_closed_ticket() {
        let (service, http) = service();
        http.respond_json(200, &dto(2, TicketStatus::Closed, None));

        let open = ticket(2, TicketStatus::Open);
        let closed = block_on(service.close_ticket(&open)).unwrap();

        assert_eq!(closed.status, TicketStatus::Closed);
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
    fn open_success_returns_open_ticket() {
        let (service, http) = service();
        http.respond_json(200, &dto(7, TicketStatus::Open, None));

        let closed = ticket(7, TicketStatus::Closed);
        let opened = block_on(service.open_ticket(&closed)).unwrap();

        assert_eq!(opened.status, TicketStatus::Open);
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
            Err(ApiError::Validation(ValidationError::AlreadyClosed(
                TicketId(2)
            )))
        );
        assert_eq!(
            block_on(service.open_ticket(&ticket(2, TicketStatus::Open))),
            Err(ApiError::Validation(ValidationError::AlreadyOpen(
                TicketId(2)
            )))
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
        http.respond_json(200, &dto(2, TicketStatus::InProgress, Some(5)));

        let alice = user(5, "Alice", Role::Agent);
        let assigned = block_on(service.assign_agent(&ticket(2, TicketStatus::Open), &alice));

        let assigned = assigned.unwrap();
        assert_eq!(assigned.assigned_agent, Some(UserId(5)));
        assert_eq!(assigned.status, TicketStatus::InProgress);
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
    fn apply_discount_returns_tenant_state() {
        let (service, http) = service();
        let tenant = TenantDto {
            tenant_id: 1,
            tenant_name: "Acme".to_owned(),
            discount_percent: 10,
            open_tickets: 6,
        };
        http.respond_json(200, &tenant);

        let tenant = block_on(service.apply_discount(TenantId(1))).unwrap();

        assert_eq!(tenant.discount_percent, 10);
        assert_eq!(
            http.requests(),
            [HttpRequest::post("/api/tenants/1/discount", None)]
        );
    }
}
