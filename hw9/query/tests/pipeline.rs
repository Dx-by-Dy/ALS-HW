//! Асинхронный конвейер целиком: команды идут в актор командной стороны, проекция
//! в отдельной задаче получает события и обновляет read-модель, политика скидок
//! в другой задаче отвечает на события новыми командами.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use command::{Command, CommandActorHandle, CommandError};
use framework::cqrs::{
    command_bus::CommandBus,
    envelope::Version,
    projection::Projection,
    query::QueryHandler,
    runner::{run_policy, run_projection},
};
use query::{
    ReadModel,
    queries::{
        ActiveTicketsOfAgent, GetTenant, GetTicket, TicketsClosedSince, UnassignedTicketsOfTenant,
    },
};
use ticketing::application::{
    clock::ManualClock,
    commands::{
        AssignAgentCommand, CloseTicketCommand, CreateTicketCommand, RegisterTenantCommand,
        RegisterUserCommand, ReopenTicketCommand,
    },
    discount_policy::DiscountPolicy,
    error::ServiceError,
    ticket_service::TicketService,
};
use ticketing::domain::{
    DomainEvent,
    tenant::{DiscountRate, TenantError, TenantId},
    ticket::{CloseReason, INACTIVITY_TIMEOUT_SECS, TicketId, TicketStatus},
    time::Timestamp,
    user::{Role, UserId},
};

const ACME: TenantId = TenantId(1);
const ALICE: UserId = UserId(1);

fn start_projection(bus: &CommandActorHandle) -> ReadModel {
    let model = ReadModel::default();
    let (bus, projected) = (bus.clone(), model.clone());
    tokio::spawn(async move { run_projection::<DomainEvent, _, _>(&bus, &projected).await });
    model
}

async fn send(bus: &CommandActorHandle, command: impl Into<Command> + Send) -> Version {
    bus.send(command).await.unwrap().version
}

async fn acme_with_alice(bus: &CommandActorHandle) -> Version {
    send(bus, RegisterTenantCommand::new("Acme")).await;
    send(bus, RegisterUserCommand::new("Alice", Role::Agent)).await
}

async fn create_ticket(bus: &CommandActorHandle) -> Version {
    send(
        bus,
        CreateTicketCommand::new("Billing", "Wrong invoice", ACME),
    )
    .await
}

#[tokio::test]
async fn read_model_follows_commands() {
    let bus = command::spawn(TicketService::new(ManualClock::default()));
    let model = start_projection(&bus);
    acme_with_alice(&bus).await;
    create_ticket(&bus).await;
    create_ticket(&bus).await;
    let assign = AssignAgentCommand {
        ticket_id: TicketId(1),
        agent_id: ALICE,
    };
    let version = send(&bus, assign).await;

    // Команда завершилась, но проекция догоняет асинхронно: ждём версию команды.
    assert_eq!(model.wait_until(version).await, version);
    let Ok(active) = model.handle(ActiveTicketsOfAgent(ALICE)).await;
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].status, TicketStatus::InProgress);
    let Ok(unassigned) = model.handle(UnassignedTicketsOfTenant(ACME)).await;
    assert_eq!(unassigned[0].ticket_id, TicketId(2));
    let Ok(acme) = model.handle(GetTenant(ACME)).await;
    assert_eq!(acme.unwrap().open_tickets, 2);
}

#[tokio::test]
async fn late_projection_catches_up_from_history() {
    let clock = ManualClock::default();
    let bus = command::spawn(TicketService::new(clock.clone()));
    acme_with_alice(&bus).await;
    create_ticket(&bus).await;
    clock.advance(INACTIVITY_TIMEOUT_SECS);
    let close = CloseTicketCommand {
        ticket_id: TicketId(1),
        reason: CloseReason::Resolved,
    };
    let version = send(&bus, close).await;

    let model = start_projection(&bus);
    model.wait_until(version).await;
    assert_eq!(model.version().await, Version(4));
    let since = Timestamp(INACTIVITY_TIMEOUT_SECS);
    let Ok(closed) = model.handle(TicketsClosedSince(since)).await;
    assert_eq!(closed.len(), 1);
    let Ok(ticket) = model.handle(GetTicket(TicketId(1))).await;
    assert_eq!(ticket.unwrap().closed_at, Some(since));
}

#[tokio::test]
async fn closing_inactive_tickets_reaches_read_model() {
    let clock = ManualClock::default();
    let bus = command::spawn(TicketService::new(clock.clone()));
    let model = start_projection(&bus);
    acme_with_alice(&bus).await;
    create_ticket(&bus).await;
    create_ticket(&bus).await;
    clock.advance(INACTIVITY_TIMEOUT_SECS);

    let version = send(&bus, Command::CloseInactiveTickets).await;
    model.wait_until(version).await;
    let Ok(closed) = model.handle(TicketsClosedSince(Timestamp(0))).await;
    assert_eq!(closed.len(), 2);
    let Ok(acme) = model.handle(GetTenant(ACME)).await;
    assert_eq!(acme.unwrap().open_tickets, 0);
}

/// Запускает политику скидок; все отказы, кроме «скидка не изменилась», копит для проверки.
fn start_discount_policy(bus: &CommandActorHandle) -> Arc<Mutex<Vec<CommandError>>> {
    let unexpected = Arc::new(Mutex::new(Vec::new()));
    let (bus, errors) = (bus.clone(), unexpected.clone());
    tokio::spawn(async move {
        run_policy::<DomainEvent, _, _, _>(&bus, Version(0), &DiscountPolicy, &bus, |e| {
            if !matches!(
                e,
                CommandError::Rejected(ServiceError::Tenant(TenantError::DiscountUnchanged(_)))
            ) {
                errors.lock().unwrap().push(e);
            }
        })
        .await
    });
    unexpected
}

/// Ждёт, пока скидка Acme в read-модели станет `expected`.
async fn discount_becomes(model: &ReadModel, expected: DiscountRate) {
    let wait = async {
        loop {
            let version = model.version().await;
            let Ok(acme) = model.handle(GetTenant(ACME)).await;
            if acme.is_some_and(|t| t.discount_rate == expected) {
                return;
            }
            model.wait_until(version.next()).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(5), wait)
        .await
        .unwrap_or_else(|_| panic!("discount did not become {expected}"));
}

#[tokio::test]
async fn discount_policy_follows_open_tickets() {
    let bus = command::spawn(TicketService::new(ManualClock::default()));
    let model = start_projection(&bus);
    let unexpected = start_discount_policy(&bus);
    acme_with_alice(&bus).await;

    for _ in 0..=DiscountRate::THRESHOLD {
        create_ticket(&bus).await;
    }
    discount_becomes(&model, DiscountRate::STANDARD).await;

    let close = CloseTicketCommand {
        ticket_id: TicketId(1),
        reason: CloseReason::Resolved,
    };
    send(&bus, close).await;
    discount_becomes(&model, DiscountRate::NONE).await;

    send(
        &bus,
        ReopenTicketCommand {
            ticket_id: TicketId(1),
        },
    )
    .await;
    discount_becomes(&model, DiscountRate::STANDARD).await;
    assert!(unexpected.lock().unwrap().is_empty());
}
