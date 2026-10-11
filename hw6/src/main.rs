use hw6::application::{
    clock::ManualClock,
    commands::{
        ApplyDiscountCommand, AssignAgentCommand, CreateTicketCommand, RegisterTenantCommand,
        RegisterUserCommand,
    },
    ticket_service::TicketService,
};
use hw6::domain::user::Role;

const DAY: u64 = 24 * 60 * 60;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let clock = ManualClock::default();
    let mut service = TicketService::new(clock.clone());

    let tenant = service.register_tenant(RegisterTenantCommand::new("Acme"))?;
    println!("{tenant}");
    let tenant_id = tenant.tenant_id();

    let alice = service.register_user(RegisterUserCommand::new("Alice", Role::Agent))?;
    let bob = service.register_user(RegisterUserCommand::new("Bob", Role::Customer))?;
    println!("{alice}\n{bob}");

    println!("\n== create tickets");
    let mut tickets = Vec::new();
    for description in ["Problem description", "VPN is down", "Cannot log in"] {
        let command = CreateTicketCommand::new("Technical Issue", description, tenant_id);
        let created = service.handle_create_ticket(command)?;
        println!("{created}");
        println!("Ticket created with ID: {}", created.ticket_id());
        tickets.push(created.ticket_id());
    }

    println!("\n== assign agent");
    println!(
        "{}",
        service.handle_assign_agent(AssignAgentCommand {
            ticket_id: tickets[0],
            agent_id: alice.user_id()
        })?
    );
    if let Err(e) = service.handle_assign_agent(AssignAgentCommand {
        ticket_id: tickets[1],
        agent_id: bob.user_id(),
    }) {
        println!("rejected: {e}");
    }

    println!("\n== discount with {} open tickets", tickets.len());
    println!(
        "{}",
        service.handle_apply_discount(ApplyDiscountCommand { tenant_id })?
    );

    println!("\n== 3 days later: agent picks up {}", tickets[1]);
    clock.advance(3 * DAY);
    println!(
        "{}",
        service.handle_assign_agent(AssignAgentCommand {
            ticket_id: tickets[1],
            agent_id: alice.user_id()
        })?
    );

    println!("\n== 5 more days later: close inactive tickets");
    clock.advance(5 * DAY);
    for closed in service.close_inactive_tickets()? {
        println!("{closed}");
    }

    println!("\n== discount is recalculated");
    println!(
        "{}",
        service.handle_apply_discount(ApplyDiscountCommand { tenant_id })?
    );

    println!("\n== state recovered from event streams");
    for ticket_id in tickets {
        if let Some(ticket) = service.ticket(ticket_id)? {
            println!(
                "{ticket_id}: {} (agent: {:?})",
                ticket.status(),
                ticket.assigned_agent()
            );
        }
    }
    if let Some(tenant) = service.tenant(tenant_id)? {
        println!("{tenant_id}: discount {}", tenant.discount_rate());
    }

    Ok(())
}
