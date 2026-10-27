use ticketing::application::{
    clock::Clock,
    commands::{
        ApplyDiscountCommand, AssignAgentCommand, CloseTicketCommand, CreateTicketCommand,
        RegisterTenantCommand, RegisterUserCommand,
    },
    error::ServiceError,
    ticket_service::TicketService,
};
use ticketing::domain::{ticket::CloseReason, user::Role};

/// Демо-данные: у Acme шесть открытых тикетов (скидка действует, закрытие любого её снимет)
/// и один закрытый, у Globex — два открытых.
pub async fn seed<C: Clock>(service: &mut TicketService<C>) -> Result<(), ServiceError> {
    let acme = service
        .register_tenant(RegisterTenantCommand::new("Acme"))
        .await?
        .tenant_id();
    let globex = service
        .register_tenant(RegisterTenantCommand::new("Globex"))
        .await?
        .tenant_id();

    let alice = service
        .register_user(RegisterUserCommand::new("Alice", Role::Agent))
        .await?
        .user_id();
    service
        .register_user(RegisterUserCommand::new("Carol", Role::Agent))
        .await?;
    service
        .register_user(RegisterUserCommand::new("Bob", Role::Customer))
        .await?;
    service
        .register_user(RegisterUserCommand::new("Dave", Role::Admin))
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
        ids.push(service.handle_create_ticket(command).await?.ticket_id());
    }
    for (category, description) in [("Billing", "Need a VAT invoice"), ("Access", "Reset 2FA")] {
        service
            .handle_create_ticket(CreateTicketCommand::new(category, description, globex))
            .await?;
    }

    service
        .handle_assign_agent(AssignAgentCommand {
            ticket_id: ids[0],
            agent_id: alice,
        })
        .await?;
    service
        .handle_close_ticket(CloseTicketCommand {
            ticket_id: ids[6],
            reason: CloseReason::Resolved,
        })
        .await?;
    service
        .handle_apply_discount(ApplyDiscountCommand { tenant_id: acme })
        .await?;
    Ok(())
}
