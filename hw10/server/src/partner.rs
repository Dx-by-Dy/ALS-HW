//! Open-Host Service для партнёров: `/partner/v1/...` на опубликованном языке `partner_api`.
//!
//! Партнёр узнаётся по токену в `Authorization: Bearer`. Он видит только переданные ему
//! тикеты: о чужих сервис отвечает 404, как будто их нет. Отчёты партнёра становятся
//! командами агрегата `Ticket`, который сам проверяет, его ли это тикет и можно ли его закрыть.
//! Ответ на отчёт приходит, когда read-модель уже его показывает: следующий `GET` его увидит.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json,
    extract::{FromRequestParts, Path, State},
    http::{StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use command::CommandError;
use framework::cqrs::{command_bus::CommandBus, envelope::Version};
use partner_api::{
    ErrorCode, PartnerError, PartnerTicket, PartnerTicketStatus, PartnerUpdate,
    TicketUpdateRequest, UpdateStatus,
};
use query::{queries, views::TicketView};
use ticketing::application::{
    commands::{ReportPartnerProgressCommand, ResolveByPartnerCommand},
    error::ServiceError,
};
use ticketing::domain::{
    partner::PartnerId,
    ticket::{CloseReason, TicketError, TicketId, TicketStatus},
};

use crate::{AppState, Shared, queries::ask};

/// Партнёр, от имени которого пришёл запрос.
pub struct PartnerAuth(pub PartnerId);

impl FromRequestParts<Arc<AppState>> for PartnerAuth {
    type Rejection = PartnerApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, PartnerApiError> {
        parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .and_then(|token| state.external.partner_tokens.get(token.trim()))
            .map(|&partner_id| Self(partner_id))
            .ok_or_else(|| {
                PartnerApiError::new(
                    StatusCode::UNAUTHORIZED,
                    ErrorCode::Unauthorized,
                    "a valid partner token is required",
                )
            })
    }
}

/// Ошибка Open-Host Service: HTTP-статус и [`PartnerError`] опубликованного языка.
#[derive(Debug)]
pub struct PartnerApiError {
    status: StatusCode,
    body: PartnerError,
}

impl PartnerApiError {
    fn new(status: StatusCode, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            status,
            body: PartnerError {
                code,
                message: message.into(),
            },
        }
    }

    fn not_handed(ticket_id: TicketId) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            format!("{ticket_id} is not handed to you"),
        )
    }
}

impl IntoResponse for PartnerApiError {
    fn into_response(self) -> Response {
        let mut response = (self.status, Json(self.body)).into_response();
        if self.status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                header::HeaderValue::from_static("Bearer"),
            );
        }
        response
    }
}

impl From<CommandError> for PartnerApiError {
    fn from(e: CommandError) -> Self {
        let message = e.to_string();
        let (status, code) = match e {
            CommandError::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::Unavailable),
            CommandError::Rejected(ServiceError::Ticket(e)) => match e {
                TicketError::NotFound | TicketError::NotWithPartner(_) => {
                    (StatusCode::NOT_FOUND, ErrorCode::NotFound)
                }
                TicketError::EmptyComment => (StatusCode::UNPROCESSABLE_ENTITY, ErrorCode::Invalid),
                _ => (StatusCode::CONFLICT, ErrorCode::Closed),
            },
            CommandError::Rejected(_) => (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::Unavailable),
        };
        Self::new(status, code, message)
    }
}

type PartnerResult<T> = Result<Json<T>, PartnerApiError>;

fn status(ticket: &TicketView) -> PartnerTicketStatus {
    match (ticket.status, ticket.close_reason) {
        (TicketStatus::Closed, Some(CloseReason::Inactivity)) => {
            PartnerTicketStatus::ClosedForInactivity
        }
        (TicketStatus::Closed, _) => PartnerTicketStatus::Resolved,
        (TicketStatus::Open | TicketStatus::InProgress, _) => PartnerTicketStatus::Open,
    }
}

/// Тикет на опубликованном языке. Внутренние подробности — агенты, напоминания, ресурсы —
/// партнёру не показываются.
fn partner_ticket(ticket: &TicketView, customers: &HashMap<usize, String>) -> PartnerTicket {
    PartnerTicket {
        ticket_id: ticket.ticket_id.0,
        category: ticket.category.clone(),
        description: ticket.description.clone(),
        customer: customers
            .get(&ticket.tenant_id.0)
            .cloned()
            .unwrap_or_default(),
        status: status(ticket),
        handed_over_at: ticket.partner.map_or(0, |(_, at)| at.0),
        due_at: ticket.sla.due_at().0,
        closed_at: ticket.closed_at.map(|at| at.0),
        updates: ticket
            .partner_updates
            .iter()
            .map(|u| PartnerUpdate {
                comment: u.comment.clone(),
                at: u.at.0,
            })
            .collect(),
    }
}

async fn customers(state: &AppState) -> HashMap<usize, String> {
    ask(state, queries::ListTenants)
        .await
        .into_iter()
        .map(|t| (t.tenant_id.0, t.tenant_name))
        .collect()
}

/// Тикет, если он передан этому партнёру.
async fn own_ticket(
    state: &AppState,
    partner_id: PartnerId,
    ticket_id: TicketId,
) -> Result<TicketView, PartnerApiError> {
    ask(state, queries::GetTicket(ticket_id))
        .await
        .filter(|t| t.partner.map(|(p, _)| p) == Some(partner_id))
        .ok_or_else(|| PartnerApiError::not_handed(ticket_id))
}

pub async fn list_tickets(
    State(state): Shared,
    PartnerAuth(partner_id): PartnerAuth,
) -> Json<Vec<PartnerTicket>> {
    let tickets = ask(&state, queries::TicketsOfPartner(partner_id)).await;
    let customers = customers(&state).await;
    Json(
        tickets
            .iter()
            .map(|t| partner_ticket(t, &customers))
            .collect(),
    )
}

pub async fn get_ticket(
    State(state): Shared,
    PartnerAuth(partner_id): PartnerAuth,
    Path(id): Path<usize>,
) -> PartnerResult<PartnerTicket> {
    let ticket = own_ticket(&state, partner_id, TicketId(id)).await?;
    Ok(Json(partner_ticket(&ticket, &customers(&state).await)))
}

/// Отчёт партнёра. `resolved` с комментарием — это отчёт и закрытие: сначала записывается
/// комментарий, потом тикет закрывается.
pub async fn post_update(
    State(state): Shared,
    PartnerAuth(partner_id): PartnerAuth,
    Path(id): Path<usize>,
    Json(request): Json<TicketUpdateRequest>,
) -> PartnerResult<PartnerTicket> {
    let ticket_id = TicketId(id);
    own_ticket(&state, partner_id, ticket_id).await?;
    let has_comment = !request.comment.trim().is_empty();
    let report = ReportPartnerProgressCommand {
        ticket_id,
        partner_id,
        comment: request.comment,
    };
    let mut version = match request.status {
        UpdateStatus::InProgress => state.commands.send(report).await?.version,
        UpdateStatus::Resolved if has_comment => state.commands.send(report).await?.version,
        UpdateStatus::Resolved => Version(0),
    };
    if request.status == UpdateStatus::Resolved {
        let resolve = ResolveByPartnerCommand {
            ticket_id,
            partner_id,
        };
        version = state.commands.send(resolve).await?.version;
    }
    // Read-your-writes: отвечаем, когда read-модель уже показывает отчёт.
    let _ = tokio::time::timeout(state.long_poll, state.read_model.wait_until(version)).await;
    let ticket = own_ticket(&state, partner_id, ticket_id).await?;
    Ok(Json(partner_ticket(&ticket, &customers(&state).await)))
}
