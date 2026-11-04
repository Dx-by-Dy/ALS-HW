use ticketing::application::commands::{
    ApplyDiscountCommand, AssignAgentCommand, CloseTicketCommand, CreateTicketCommand,
    RegisterTenantCommand, RegisterUserCommand, ReopenTicketCommand,
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
    ApplyDiscount(ApplyDiscountCommand),
    /// Закрыть все тикеты, у которых истёк таймаут неактивности.
    CloseInactiveTickets,
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
    ApplyDiscount(ApplyDiscountCommand),
);
