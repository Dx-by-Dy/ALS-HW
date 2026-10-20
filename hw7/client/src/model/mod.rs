//! Model: данные приложения, правила допустимости команд и сервис связи с бэкендом.
//!
//! Источник истины — сервер. Клиентская модель проверяет команды заранее, чтобы не слать
//! заведомо отклонённые запросы, а состояние после команды берёт из ответа сервера.

pub mod http;
#[cfg(test)]
pub mod mock_http;
pub mod ticket_api;

use std::error::Error;
use std::fmt;

use api::{TenantDto, TicketDto, UserDto};

pub use api::{Role, TicketStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TicketId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UserId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TenantId(pub usize);

impl fmt::Display for TicketId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ticket-{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    pub ticket_id: TicketId,
    pub category: String,
    pub description: String,
    pub status: TicketStatus,
    pub assigned_agent: Option<UserId>,
    pub tenant_id: TenantId,
}

impl Ticket {
    /// Открыть можно только закрытый тикет.
    pub fn validate_open(&self) -> Result<(), ValidationError> {
        match self.status {
            TicketStatus::Closed => Ok(()),
            TicketStatus::Open | TicketStatus::InProgress => {
                Err(ValidationError::AlreadyOpen(self.ticket_id))
            }
        }
    }

    /// Закрыть можно только открытый тикет.
    pub fn validate_close(&self) -> Result<(), ValidationError> {
        match self.status {
            TicketStatus::Closed => Err(ValidationError::AlreadyClosed(self.ticket_id)),
            TicketStatus::Open | TicketStatus::InProgress => Ok(()),
        }
    }

    /// Назначить можно только агента, только на открытый тикет и только другого агента.
    pub fn validate_assign(&self, agent: &User) -> Result<(), ValidationError> {
        self.validate_close()?;
        if agent.role != Role::Agent {
            return Err(ValidationError::NotAnAgent {
                user_name: agent.user_name.clone(),
                role: agent.role,
            });
        }
        if self.assigned_agent == Some(agent.user_id) {
            return Err(ValidationError::AlreadyAssigned {
                user_name: agent.user_name.clone(),
                ticket_id: self.ticket_id,
            });
        }
        Ok(())
    }
}

impl From<TicketDto> for Ticket {
    fn from(dto: TicketDto) -> Self {
        Self {
            ticket_id: TicketId(dto.ticket_id),
            category: dto.category,
            description: dto.description,
            status: dto.status,
            assigned_agent: dto.assigned_agent.map(UserId),
            tenant_id: TenantId(dto.tenant_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub user_id: UserId,
    pub user_name: String,
    pub role: Role,
}

impl From<UserDto> for User {
    fn from(dto: UserDto) -> Self {
        Self {
            user_id: UserId(dto.user_id),
            user_name: dto.user_name,
            role: dto.role,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tenant {
    pub tenant_id: TenantId,
    pub tenant_name: String,
    pub discount_percent: u8,
    pub open_tickets: usize,
}

impl From<TenantDto> for Tenant {
    fn from(dto: TenantDto) -> Self {
        Self {
            tenant_id: TenantId(dto.tenant_id),
            tenant_name: dto.tenant_name,
            discount_percent: dto.discount_percent,
            open_tickets: dto.open_tickets,
        }
    }
}

/// Команда недопустима при текущем состоянии модели.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    AlreadyOpen(TicketId),
    AlreadyClosed(TicketId),
    NotAnAgent {
        user_name: String,
        role: Role,
    },
    AlreadyAssigned {
        user_name: String,
        ticket_id: TicketId,
    },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyOpen(id) => write!(f, "{id} is already open"),
            Self::AlreadyClosed(id) => write!(f, "{id} is already closed"),
            Self::NotAnAgent { user_name, role } => {
                write!(
                    f,
                    "{user_name} has role {role}, only agents can be assigned"
                )
            }
            Self::AlreadyAssigned {
                user_name,
                ticket_id,
            } => write!(f, "{user_name} is already assigned to {ticket_id}"),
        }
    }
}

impl Error for ValidationError {}

#[cfg(test)]
pub mod fixtures {
    use super::*;

    pub fn ticket(id: usize, status: TicketStatus) -> Ticket {
        Ticket {
            ticket_id: TicketId(id),
            category: "Technical Issue".to_owned(),
            description: "VPN is down".to_owned(),
            status,
            assigned_agent: None,
            tenant_id: TenantId(1),
        }
    }

    pub fn user(id: usize, name: &str, role: Role) -> User {
        User {
            user_id: UserId(id),
            user_name: name.to_owned(),
            role,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{ticket, user};
    use super::*;

    #[test]
    fn only_closed_ticket_can_be_opened() {
        assert_eq!(ticket(1, TicketStatus::Closed).validate_open(), Ok(()));
        for status in [TicketStatus::Open, TicketStatus::InProgress] {
            assert_eq!(
                ticket(1, status).validate_open(),
                Err(ValidationError::AlreadyOpen(TicketId(1)))
            );
        }
    }

    #[test]
    fn only_open_ticket_can_be_closed() {
        for status in [TicketStatus::Open, TicketStatus::InProgress] {
            assert_eq!(ticket(1, status).validate_close(), Ok(()));
        }
        assert_eq!(
            ticket(1, TicketStatus::Closed).validate_close(),
            Err(ValidationError::AlreadyClosed(TicketId(1)))
        );
    }

    #[test]
    fn only_agent_can_be_assigned() {
        let open = ticket(1, TicketStatus::Open);
        assert_eq!(open.validate_assign(&user(1, "Alice", Role::Agent)), Ok(()));
        for role in [Role::Customer, Role::Admin] {
            assert_eq!(
                open.validate_assign(&user(2, "Bob", role)),
                Err(ValidationError::NotAnAgent {
                    user_name: "Bob".to_owned(),
                    role
                })
            );
        }
    }

    #[test]
    fn closed_ticket_or_same_agent_blocks_assignment() {
        let alice = user(1, "Alice", Role::Agent);
        assert_eq!(
            ticket(1, TicketStatus::Closed).validate_assign(&alice),
            Err(ValidationError::AlreadyClosed(TicketId(1)))
        );
        let assigned = Ticket {
            assigned_agent: Some(UserId(1)),
            ..ticket(1, TicketStatus::InProgress)
        };
        assert_eq!(
            assigned.validate_assign(&alice),
            Err(ValidationError::AlreadyAssigned {
                user_name: "Alice".to_owned(),
                ticket_id: TicketId(1)
            })
        );
    }
}
