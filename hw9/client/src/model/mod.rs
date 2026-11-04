//! Model: данные приложения, правила допустимости команд и сервис связи с бэкендом.
//!
//! Данные — DTO из общего crate `api`, отдельной копии структур на клиенте нет.
//! Источник истины — сервер: клиент проверяет команды заранее, чтобы не слать
//! заведомо отклонённые запросы, а состояние держит в [`replica::Replica`] — копии
//! read-модели сервера, которую обновляет long polling.

pub mod backoff;
pub mod http;
#[cfg(test)]
pub mod mock_http;
pub mod replica;
pub mod ticket_api;

use std::error::Error;
use std::fmt;

use api::{Role, TicketDto, TicketStatus, UserDto};

/// Правила допустимости команд над тикетом. Данные модели — DTO из `api`, а правила
/// клиент добавляет к ним сам: по ним ViewModel решает, доступна ли команда.
pub trait TicketRules {
    /// Открыть можно только закрытый тикет.
    fn validate_open(&self) -> Result<(), ValidationError>;
    /// Закрыть можно только открытый тикет.
    fn validate_close(&self) -> Result<(), ValidationError>;
    /// Назначить можно только агента, только на открытый тикет и только другого агента.
    fn validate_assign(&self, agent: &UserDto) -> Result<(), ValidationError>;
}

impl TicketRules for TicketDto {
    fn validate_open(&self) -> Result<(), ValidationError> {
        match self.status {
            TicketStatus::Closed => Ok(()),
            TicketStatus::Open | TicketStatus::InProgress => {
                Err(ValidationError::AlreadyOpen(self.ticket_id))
            }
        }
    }

    fn validate_close(&self) -> Result<(), ValidationError> {
        match self.status {
            TicketStatus::Closed => Err(ValidationError::AlreadyClosed(self.ticket_id)),
            TicketStatus::Open | TicketStatus::InProgress => Ok(()),
        }
    }

    fn validate_assign(&self, agent: &UserDto) -> Result<(), ValidationError> {
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

/// Команда недопустима при текущем состоянии модели.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    AlreadyOpen(usize),
    AlreadyClosed(usize),
    NotAnAgent { user_name: String, role: Role },
    AlreadyAssigned { user_name: String, ticket_id: usize },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyOpen(id) => write!(f, "ticket-{id} is already open"),
            Self::AlreadyClosed(id) => write!(f, "ticket-{id} is already closed"),
            Self::NotAnAgent { user_name, role } => {
                write!(
                    f,
                    "{user_name} has role {role}, only agents can be assigned"
                )
            }
            Self::AlreadyAssigned {
                user_name,
                ticket_id,
            } => write!(f, "{user_name} is already assigned to ticket-{ticket_id}"),
        }
    }
}

impl Error for ValidationError {}

#[cfg(test)]
pub mod fixtures {
    use super::*;
    use api::ChangesDto;

    pub fn ticket(id: usize, status: TicketStatus) -> TicketDto {
        TicketDto {
            ticket_id: id,
            category: "Technical Issue".to_owned(),
            description: "VPN is down".to_owned(),
            status,
            assigned_agent: None,
            tenant_id: 1,
            created_at: 0,
            closed_at: None,
        }
    }

    pub fn user(id: usize, name: &str, role: Role) -> UserDto {
        UserDto {
            user_id: id,
            user_name: name.to_owned(),
            role,
        }
    }

    pub fn changes(epoch: u64, version: u64, full: bool, tickets: Vec<TicketDto>) -> ChangesDto {
        ChangesDto {
            epoch,
            version,
            full,
            tickets,
            ..ChangesDto::default()
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
                Err(ValidationError::AlreadyOpen(1))
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
            Err(ValidationError::AlreadyClosed(1))
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
            Err(ValidationError::AlreadyClosed(1))
        );
        let assigned = TicketDto {
            assigned_agent: Some(1),
            ..ticket(1, TicketStatus::InProgress)
        };
        assert_eq!(
            assigned.validate_assign(&alice),
            Err(ValidationError::AlreadyAssigned {
                user_name: "Alice".to_owned(),
                ticket_id: 1
            })
        );
    }
}
