use ticketing::application::{
    commands::{
        ApplyDiscountCommand, AssignAgentCommand, CloseTicketCommand, CreateTicketCommand,
        RegisterTenantCommand, RegisterUserCommand, ReopenTicketCommand, SendReminderCommand,
    },
    ticket_lifecycle::LifecycleCommand,
};

/// Любая команда, которую принимает командная сторона.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    RegisterTenant(RegisterTenantCommand),
    RegisterUser(RegisterUserCommand),
    CreateTicket(CreateTicketCommand),
    AssignAgent(AssignAgentCommand),
    CloseTicket(CloseTicketCommand),
    ReopenTicket(ReopenTicketCommand),
    SendReminder(SendReminderCommand),
    ApplyDiscount(ApplyDiscountCommand),
}

impl From<LifecycleCommand> for Command {
    fn from(command: LifecycleCommand) -> Self {
        match command {
            LifecycleCommand::SendReminder(c) => Self::SendReminder(c),
            LifecycleCommand::CloseTicket(c) => Self::CloseTicket(c),
        }
    }
}

macro_rules! impl_from_command {
    ($($variant:ident($command:ty)),* $(,)?) => {
        $(
            impl From<$command> for Command {
                fn from(command: $command) -> Self {
                    Self::$variant(command)
                }
            }
        )*
    };
}

impl_from_command!(
    RegisterTenant(RegisterTenantCommand),
    RegisterUser(RegisterUserCommand),
    CreateTicket(CreateTicketCommand),
    AssignAgent(AssignAgentCommand),
    CloseTicket(CloseTicketCommand),
    ReopenTicket(ReopenTicketCommand),
    SendReminder(SendReminderCommand),
    ApplyDiscount(ApplyDiscountCommand),
);
