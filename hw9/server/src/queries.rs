//! Обработчики запросов: читают только read-модель и ничего не меняют.

use std::convert::Infallible;

use api::{ChangesDto, TenantDto, TicketDto, UserDto};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use framework::cqrs::{
    envelope::Version,
    projection::Projection,
    query::{Query as ReadQuery, QueryHandler},
};
use query::{ReadModel, queries, views::TicketView};
use serde::Deserialize;
use ticketing::domain::{tenant::TenantId, ticket::TicketId, time::Timestamp, user::UserId};

use crate::{ApiError, ApiResult, AppState, Shared, dto};

const DAY_SECS: u64 = 24 * 60 * 60;

async fn ask<Q>(state: &AppState, query: Q) -> Q::Output
where
    Q: ReadQuery,
    ReadModel: QueryHandler<Q, Error = Infallible>,
{
    let Ok(answer) = state.read_model.handle(query).await;
    answer
}

fn tickets(views: Vec<TicketView>) -> Json<Vec<TicketDto>> {
    Json(views.iter().map(dto::ticket_dto).collect())
}

#[derive(Debug, Deserialize)]
pub struct TicketsParams {
    status: Option<api::TicketStatus>,
    tenant: Option<usize>,
}

pub async fn list_tickets(
    State(state): Shared,
    Query(params): Query<TicketsParams>,
) -> Json<Vec<TicketDto>> {
    let query = queries::ListTickets {
        status: params.status.map(dto::ticket_status),
        tenant_id: params.tenant.map(TenantId),
    };
    tickets(ask(&state, query).await)
}

pub async fn get_ticket(State(state): Shared, Path(id): Path<usize>) -> ApiResult<TicketDto> {
    let ticket_id = TicketId(id);
    let ticket = ask(&state, queries::GetTicket(ticket_id))
        .await
        .ok_or_else(|| ApiError::not_found(format!("{ticket_id} does not exist")))?;
    Ok(Json(dto::ticket_dto(&ticket)))
}

pub async fn active_tickets_of_agent(
    State(state): Shared,
    Path(id): Path<usize>,
) -> Json<Vec<TicketDto>> {
    tickets(ask(&state, queries::ActiveTicketsOfAgent(UserId(id))).await)
}

pub async fn unassigned_tickets_of_tenant(
    State(state): Shared,
    Path(id): Path<usize>,
) -> Json<Vec<TicketDto>> {
    tickets(ask(&state, queries::UnassignedTicketsOfTenant(TenantId(id))).await)
}

#[derive(Debug, Deserialize)]
pub struct ClosedParams {
    days: u64,
}

/// Read-модель не знает текущего времени: «последние N дней» превращаются в момент `since` здесь.
pub async fn tickets_closed_within(
    State(state): Shared,
    Query(params): Query<ClosedParams>,
) -> Json<Vec<TicketDto>> {
    let window = params.days.saturating_mul(DAY_SECS);
    let since = Timestamp(state.clock.now().0.saturating_sub(window));
    tickets(ask(&state, queries::TicketsClosedSince(since)).await)
}

pub async fn list_tenants(State(state): Shared) -> Json<Vec<TenantDto>> {
    let tenants = ask(&state, queries::ListTenants).await;
    Json(tenants.iter().map(dto::tenant_dto).collect())
}

pub async fn get_tenant(State(state): Shared, Path(id): Path<usize>) -> ApiResult<TenantDto> {
    let tenant_id = TenantId(id);
    let tenant = ask(&state, queries::GetTenant(tenant_id))
        .await
        .ok_or_else(|| ApiError::not_found(format!("{tenant_id} does not exist")))?;
    Ok(Json(dto::tenant_dto(&tenant)))
}

pub async fn list_users(State(state): Shared) -> Json<Vec<UserDto>> {
    let users = ask(&state, queries::ListUsers).await;
    Json(users.iter().map(dto::user_dto).collect())
}

#[derive(Debug, Deserialize)]
pub struct ChangesParams {
    epoch: Option<u64>,
    since: Option<u64>,
}

/// Long polling. Если после `since` уже что-то изменилось, отвечает сразу; иначе ждёт,
/// пока read-модель не пройдёт `since`, но не дольше `long_poll`, и отвечает тем, что есть.
///
/// `since` может обгонять read-модель: клиент знает версию своей команды раньше, чем
/// проекция её применит. Тогда запрос просто ждёт — так клиент видит собственные изменения.
pub async fn changes(
    State(state): Shared,
    Query(params): Query<ChangesParams>,
) -> Json<ChangesDto> {
    // Без позиции или с позицией из другого запуска сервера — отдаём всё состояние сразу.
    let since = match params {
        ChangesParams {
            epoch: Some(epoch),
            since: Some(since),
        } if epoch == state.epoch => Some(Version(since)),
        _ => None,
    };
    if let Some(since) = since
        && state.read_model.version().await <= since
    {
        let next = state.read_model.wait_until(since.next());
        // Истёкший таймаут — не ошибка: просто изменений не было.
        let _ = tokio::time::timeout(state.long_poll, next).await;
    }
    let changes = ask(&state, queries::ChangesSince(since.unwrap_or_default())).await;
    Json(dto::changes_dto(state.epoch, since.is_none(), &changes))
}
