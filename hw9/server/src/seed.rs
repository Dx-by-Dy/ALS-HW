use command::{CommandActorHandle, CommandError};
use framework::cqrs::{
    command_bus::CommandBus,
    envelope::{Committed, Version},
};
use ticketing::application::commands::{
    AssignAgentCommand, CloseTicketCommand, CreateTicketCommand, RegisterTenantCommand,
    RegisterUserCommand,
};
use ticketing::domain::{
    DomainEvent,
    tenant::TenantId,
    ticket::{CloseReason, TicketId},
    user::{Role, UserId},
};

/// Демо-данные: у Acme шесть открытых тикетов (скидка действует, закрытие любого её снимет)
/// и один закрытый, у Globex — два открытых. Скидку назначает политика, а не seed.
///
/// Данные идут через шину команд, как и всё остальное, поэтому попадают в общий журнал
/// и read-модель. Возвращает версию последней команды seed.
pub async fn seed(bus: &CommandActorHandle) -> Result<Version, CommandError> {
    let acme = tenant_id(bus.send(RegisterTenantCommand::new("Acme")).await?);
    let globex = tenant_id(bus.send(RegisterTenantCommand::new("Globex")).await?);

    let alice = user_id(
        bus.send(RegisterUserCommand::new("Alice", Role::Agent))
            .await?,
    );
    bus.send(RegisterUserCommand::new("Carol", Role::Agent))
        .await?;
    bus.send(RegisterUserCommand::new("Bob", Role::Customer))
        .await?;
    bus.send(RegisterUserCommand::new("Dave", Role::Admin))
        .await?;

    let acme_tickets = [
        ("Technical Issue", "VPN is down"),
        ("Technical Issue", "Cannot log in"),
        ("Billing", "Invoice has a wrong address"),
        ("Technical Issue", "Printer on floor 3 is offline"),
        ("Access", "Need access to the reports folder"),
        ("Billing", "Duplicate charge in March"),
        ("Technical Issue", "Laptop battery drains fast"),
    ];
    let mut ids = Vec::new();
    for (category, description) in acme_tickets {
        let command = CreateTicketCommand::new(category, description, acme);
        ids.push(ticket_id(bus.send(command).await?));
    }
    for (category, description) in [("Billing", "Need a VAT invoice"), ("Access", "Reset 2FA")] {
        bus.send(CreateTicketCommand::new(category, description, globex))
            .await?;
    }

    bus.send(AssignAgentCommand {
        ticket_id: ids[0],
        agent_id: alice,
    })
    .await?;
    let closed = bus
        .send(CloseTicketCommand {
            ticket_id: ids[6],
            reason: CloseReason::Resolved,
        })
        .await?;
    Ok(closed.version)
}

/// Событие, которое записала команда регистрации или создания: ровно одно.
fn only_event(committed: Committed<DomainEvent>) -> DomainEvent {
    let [envelope] = <[_; 1]>::try_from(committed.events).expect("command stores one event");
    envelope.event
}

fn tenant_id(committed: Committed<DomainEvent>) -> TenantId {
    match only_event(committed) {
        DomainEvent::Tenant(event) => event.tenant_id(),
        other => panic!("expected a tenant event, got {other}"),
    }
}

fn user_id(committed: Committed<DomainEvent>) -> UserId {
    match only_event(committed) {
        DomainEvent::User(event) => event.user_id(),
        other => panic!("expected a user event, got {other}"),
    }
}

fn ticket_id(committed: Committed<DomainEvent>) -> TicketId {
    match only_event(committed) {
        DomainEvent::Ticket(event) => event.ticket_id(),
        other => panic!("expected a ticket event, got {other}"),
    }
}
