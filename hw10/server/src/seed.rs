use std::error::Error;
use std::fmt;

use command::CommandError;
use framework::cqrs::{
    command_bus::CommandBus,
    envelope::{Committed, Version},
};
use ticketing::application::{
    commands::{
        AssignAgentCommand, CloseTicketCommand, RegisterTenantCommand, RegisterUserCommand,
    },
    intake::{IntakeError, admit_ticket},
};
use ticketing::domain::{
    DomainEvent,
    sla::CustomerTier,
    tenant::TenantId,
    ticket::{CloseReason, TicketId},
    user::{Role, UserId},
};

use crate::App;

#[derive(Debug)]
pub enum SeedError {
    Command(CommandError),
    Intake(IntakeError),
}

impl fmt::Display for SeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Command(e) => write!(f, "seed command failed: {e}"),
            Self::Intake(e) => write!(f, "seed ticket was not admitted: {e}"),
        }
    }
}

impl Error for SeedError {}

impl From<CommandError> for SeedError {
    fn from(e: CommandError) -> Self {
        Self::Command(e)
    }
}

impl From<IntakeError> for SeedError {
    fn from(e: IntakeError) -> Self {
        Self::Intake(e)
    }
}

/// Демо-данные: у Acme (Gold) шесть открытых тикетов (скидка действует, закрытие любого
/// её снимет) и один закрытый, у Globex (Silver) — два открытых. Скидку назначает политика,
/// а не seed. Тикет Acme про принтер — `Hardware`: запчастей на складе нет, и его забирает
/// партнёр; тикет про VPN — `Network`: ресурсы есть, решают своими силами.
///
/// Данные идут тем же путём, что и запросы HTTP: тенанты получают счёт в CRM, тикеты
/// принимаются через проверку клиента. Возвращает версию последней команды seed.
pub async fn seed(app: &App) -> Result<Version, SeedError> {
    let bus = &app.commands;
    let crm = app.external.crm_adapter();
    let tenant = async |name: &str, tier| -> Result<TenantId, SeedError> {
        let id = tenant_id(bus.send(RegisterTenantCommand::new(name)).await?);
        app.external.open_account(id, name, tier);
        Ok(id)
    };
    let acme = tenant("Acme", CustomerTier::Gold).await?;
    let globex = tenant("Globex", CustomerTier::Silver).await?;

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
        ("Network", "VPN is down"),
        ("Technical Issue", "Cannot log in"),
        ("Billing", "Invoice has a wrong address"),
        ("Hardware", "Printer on floor 3 is offline"),
        ("Access", "Need access to the reports folder"),
        ("Billing", "Duplicate charge in March"),
        ("Technical Issue", "Laptop battery drains fast"),
    ];
    let mut ids = Vec::new();
    let admit = async |category: &str, description: &str, tenant_id| {
        admit_ticket(&crm, category.to_owned(), description.to_owned(), tenant_id).await
    };
    for (category, description) in acme_tickets {
        let command = admit(category, description, acme).await?;
        ids.push(ticket_id(bus.send(command).await?));
    }
    for (category, description) in [("Billing", "Need a VAT invoice"), ("Access", "Reset 2FA")] {
        bus.send(admit(category, description, globex).await?)
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
