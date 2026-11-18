//! Обработчики команд. Команда проверяется и выполняется на командной стороне сразу,
//! поэтому ошибки (404 / 409 / 422) клиент получает в ответе. При успехе ответ — версия,
//! начиная с которой результат виден в read-модели; само состояние читается запросами.

use api::{
    AssignAgentRequest, CommandAcceptedDto, CreateTenantRequest, CreateTicketRequest,
    CreateUserRequest,
};
use axum::{
    Json,
    extract::{Path, State},
};
use command::{Command, CommandError};
use framework::cqrs::{command_bus::CommandBus, envelope::Committed};
use ticketing::application::commands::{
    AssignAgentCommand, CloseTicketCommand, CreateTicketCommand, RegisterTenantCommand,
    RegisterUserCommand, ReopenTicketCommand,
};
use ticketing::domain::{
    DomainEvent,
    sla::CustomerTier,
    tenant::TenantId,
    ticket::{CloseReason, TicketId},
    user::UserId,
};

use crate::{ApiResult, Shared, dto};

pub async fn create_tenant(
    State(state): Shared,
    Json(request): Json<CreateTenantRequest>,
) -> ApiResult<CommandAcceptedDto> {
    send(&state, RegisterTenantCommand::new(request.tenant_name)).await
}

pub async fn create_user(
    State(state): Shared,
    Json(request): Json<CreateUserRequest>,
) -> ApiResult<CommandAcceptedDto> {
    let command = RegisterUserCommand::new(request.user_name, dto::role(request.role));
    send(&state, command).await
}

pub async fn create_ticket(
    State(state): Shared,
    Json(request): Json<CreateTicketRequest>,
) -> ApiResult<CommandAcceptedDto> {
    // Уровень клиента пока не спрашиваем у CRM: все клиенты считаются Bronze.
    let command = CreateTicketCommand::new(
        request.category,
        request.description,
        TenantId(request.tenant_id),
        CustomerTier::Bronze,
    );
    send(&state, command).await
}

pub async fn open_ticket(
    State(state): Shared,
    Path(id): Path<usize>,
) -> ApiResult<CommandAcceptedDto> {
    send(
        &state,
        ReopenTicketCommand {
            ticket_id: TicketId(id),
        },
    )
    .await
}

pub async fn close_ticket(
    State(state): Shared,
    Path(id): Path<usize>,
) -> ApiResult<CommandAcceptedDto> {
    let command = CloseTicketCommand {
        ticket_id: TicketId(id),
        reason: CloseReason::Resolved,
    };
    send(&state, command).await
}

pub async fn assign_agent(
    State(state): Shared,
    Path(id): Path<usize>,
    Json(request): Json<AssignAgentRequest>,
) -> ApiResult<CommandAcceptedDto> {
    let command = AssignAgentCommand {
        ticket_id: TicketId(id),
        agent_id: UserId(request.agent_id),
    };
    send(&state, command).await
}

async fn send(
    state: &crate::AppState,
    command: impl Into<Command> + Send,
) -> ApiResult<CommandAcceptedDto> {
    let committed: Result<Committed<DomainEvent>, CommandError> =
        state.commands.send(command).await;
    Ok(Json(CommandAcceptedDto {
        version: committed?.version.0,
    }))
}
