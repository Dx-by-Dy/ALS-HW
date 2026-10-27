//! HTTP-адаптер над `TicketService`: принимает команды от клиента и отдаёт состояние после них.

mod dto;
mod error;
mod seed;

use std::sync::{Arc, Mutex, MutexGuard};

use api::{AssignAgentRequest, TenantDto, TicketDto, UserDto};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use ticketing::application::{
    clock::Clock,
    commands::{ApplyDiscountCommand, AssignAgentCommand, CloseTicketCommand, ReopenTicketCommand},
    error::ServiceError,
    ticket_service::TicketService,
};
use ticketing::domain::{
    tenant::{TenantError, TenantId},
    ticket::{CloseReason, TicketId},
    user::UserId,
};

pub use error::ApiError;
pub use seed::seed;

type ApiResult<T> = Result<Json<T>, ApiError>;

pub struct AppState<C> {
    service: Mutex<TicketService<C>>,
}

impl<C> AppState<C> {
    fn service(&self) -> MutexGuard<'_, TicketService<C>> {
        self.service
            .lock()
            .expect("ticket service lock is not poisoned")
    }
}

pub fn app<C: Clock + Send + 'static>(service: TicketService<C>) -> Router {
    let state = Arc::new(AppState {
        service: Mutex::new(service),
    });
    Router::new()
        .route("/api/tickets", get(list_tickets::<C>))
        .route("/api/tickets/{id}/open", post(open_ticket::<C>))
        .route("/api/tickets/{id}/close", post(close_ticket::<C>))
        .route("/api/tickets/{id}/assign", post(assign_agent::<C>))
        .route("/api/users", get(list_users::<C>))
        .route("/api/tenants", get(list_tenants::<C>))
        .route("/api/tenants/{id}/discount", post(apply_discount::<C>))
        .with_state(state)
}

type Shared<C> = State<Arc<AppState<C>>>;

async fn list_tickets<C: Clock>(State(state): Shared<C>) -> ApiResult<Vec<TicketDto>> {
    let tickets = state.service().tickets()?;
    Ok(Json(tickets.iter().map(dto::ticket_dto).collect()))
}

async fn list_users<C: Clock>(State(state): Shared<C>) -> ApiResult<Vec<UserDto>> {
    let users = state.service().users()?;
    Ok(Json(users.iter().map(dto::user_dto).collect()))
}

async fn list_tenants<C: Clock>(State(state): Shared<C>) -> ApiResult<Vec<TenantDto>> {
    let service = state.service();
    let tenants = service.tenants()?;
    let dtos = tenants
        .iter()
        .map(|t| dto::tenant_dto(t, service.open_tickets_count(t.tenant_id())))
        .collect();
    Ok(Json(dtos))
}

async fn open_ticket<C: Clock>(
    State(state): Shared<C>,
    Path(id): Path<usize>,
) -> ApiResult<TicketDto> {
    let mut service = state.service();
    let ticket_id = TicketId(id);
    service.handle_reopen_ticket(ReopenTicketCommand { ticket_id })?;
    ticket_response(&service, ticket_id)
}

async fn close_ticket<C: Clock>(
    State(state): Shared<C>,
    Path(id): Path<usize>,
) -> ApiResult<TicketDto> {
    let mut service = state.service();
    let ticket_id = TicketId(id);
    service.handle_close_ticket(CloseTicketCommand {
        ticket_id,
        reason: CloseReason::Resolved,
    })?;
    ticket_response(&service, ticket_id)
}

async fn assign_agent<C: Clock>(
    State(state): Shared<C>,
    Path(id): Path<usize>,
    Json(request): Json<AssignAgentRequest>,
) -> ApiResult<TicketDto> {
    let mut service = state.service();
    let ticket_id = TicketId(id);
    service.handle_assign_agent(AssignAgentCommand {
        ticket_id,
        agent_id: UserId(request.agent_id),
    })?;
    ticket_response(&service, ticket_id)
}

/// Пересчитывает скидку. Для клиента операция идемпотентна: если скидка не изменилась,
/// событие не пишется, а ответ всё равно содержит текущее состояние тенанта.
async fn apply_discount<C: Clock>(
    State(state): Shared<C>,
    Path(id): Path<usize>,
) -> ApiResult<TenantDto> {
    let mut service = state.service();
    let tenant_id = TenantId(id);
    match service.handle_apply_discount(ApplyDiscountCommand { tenant_id }) {
        Ok(_) | Err(ServiceError::Tenant(TenantError::DiscountUnchanged(_))) => {}
        Err(e) => return Err(e.into()),
    }
    let tenant = service
        .tenant(tenant_id)?
        .ok_or_else(|| ApiError::not_found(format!("{tenant_id} does not exist")))?;
    Ok(Json(dto::tenant_dto(
        &tenant,
        service.open_tickets_count(tenant_id),
    )))
}

fn ticket_response<C: Clock>(
    service: &TicketService<C>,
    ticket_id: TicketId,
) -> ApiResult<TicketDto> {
    let ticket = service
        .ticket(ticket_id)?
        .ok_or_else(|| ApiError::not_found(format!("{ticket_id} does not exist")))?;
    Ok(Json(dto::ticket_dto(&ticket)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{ErrorDto, TicketStatus};
    use axum::{
        body::Body,
        http::{Request, StatusCode, header},
    };
    use http_body_util::BodyExt;
    use serde::de::DeserializeOwned;
    use ticketing::application::clock::SystemClock;
    use tower::ServiceExt;

    /// Номера тикетов и пользователей из [`seed`].
    const ASSIGNED: usize = 1;
    const OPEN: usize = 2;
    const CLOSED: usize = 7;
    const AGENT_CAROL: usize = 2;
    const CUSTOMER_BOB: usize = 3;
    const ACME: usize = 1;

    fn seeded_app() -> Router {
        let mut service = TicketService::new(SystemClock);
        seed(&mut service).unwrap();
        app(service)
    }

    async fn send<T: DeserializeOwned>(
        app: &Router,
        method: &str,
        uri: &str,
        body: Option<&str>,
    ) -> (StatusCode, T) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_owned())))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn lists_seeded_state() {
        let app = seeded_app();
        let (status, tickets): (_, Vec<TicketDto>) = send(&app, "GET", api::TICKETS, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(tickets.len(), 9);
        assert_eq!(tickets[ASSIGNED - 1].status, TicketStatus::InProgress);
        assert_eq!(tickets[CLOSED - 1].status, TicketStatus::Closed);

        let (_, users): (_, Vec<UserDto>) = send(&app, "GET", api::USERS, None).await;
        assert_eq!(users.len(), 4);

        let (_, tenants): (_, Vec<TenantDto>) = send(&app, "GET", api::TENANTS, None).await;
        assert_eq!(tenants[ACME - 1].open_tickets, 6);
        assert_eq!(tenants[ACME - 1].discount_percent, 10);
    }

    #[tokio::test]
    async fn close_then_open_round_trip() {
        let app = seeded_app();
        let (status, ticket): (_, TicketDto) =
            send(&app, "POST", &api::close_ticket(OPEN), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(ticket.status, TicketStatus::Closed);

        let (status, error): (_, ErrorDto) =
            send(&app, "POST", &api::close_ticket(OPEN), None).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error.error, "ticket-2 is closed");

        let (status, ticket): (_, TicketDto) =
            send(&app, "POST", &api::open_ticket(OPEN), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(ticket.status, TicketStatus::Open);
    }

    #[tokio::test]
    async fn opening_open_ticket_conflicts() {
        let (status, error): (_, ErrorDto) =
            send(&seeded_app(), "POST", &api::open_ticket(OPEN), None).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error.error, "ticket-2 is already open");
    }

    #[tokio::test]
    async fn assign_accepts_only_agents() {
        let app = seeded_app();
        let body = format!(r#"{{"agent_id":{AGENT_CAROL}}}"#);
        let (status, ticket): (_, TicketDto) =
            send(&app, "POST", &api::assign_agent(OPEN), Some(&body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(ticket.assigned_agent, Some(AGENT_CAROL));
        assert_eq!(ticket.status, TicketStatus::InProgress);

        let body = format!(r#"{{"agent_id":{CUSTOMER_BOB}}}"#);
        let (status, _): (_, ErrorDto) =
            send(&app, "POST", &api::assign_agent(OPEN), Some(&body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn unknown_ticket_is_not_found() {
        let (status, error): (_, ErrorDto) =
            send(&seeded_app(), "POST", &api::close_ticket(99), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(error.error, "ticket does not exist");
    }

    #[tokio::test]
    async fn discount_follows_open_tickets_and_is_idempotent() {
        let app = seeded_app();
        let (status, tenant): (_, TenantDto) =
            send(&app, "POST", &api::apply_discount(ACME), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(tenant.discount_percent, 10);

        let _: (_, TicketDto) = send(&app, "POST", &api::close_ticket(OPEN), None).await;
        let (_, tenant): (_, TenantDto) =
            send(&app, "POST", &api::apply_discount(ACME), None).await;
        assert_eq!(tenant.open_tickets, 5);
        assert_eq!(tenant.discount_percent, 0);

        let (status, _): (_, ErrorDto) = send(&app, "POST", &api::apply_discount(99), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
