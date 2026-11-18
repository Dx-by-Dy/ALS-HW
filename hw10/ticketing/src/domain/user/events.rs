use std::fmt;

use super::{Role, UserId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserEvent {
    Registered(UserRegisteredEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRegisteredEvent {
    pub user_id: UserId,
    pub user_name: String,
    pub role: Role,
}

impl UserEvent {
    pub fn user_id(&self) -> UserId {
        match self {
            Self::Registered(e) => e.user_id,
        }
    }
}

impl fmt::Display for UserEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registered(e) => {
                write!(
                    f,
                    "UserRegistered: {} '{}' as {}",
                    e.user_id, e.user_name, e.role
                )
            }
        }
    }
}
