mod command;
mod error;
mod events;

use std::fmt;

pub use command::UserCommand;
pub use error::UserError;
pub use events::{UserEvent, UserRegisteredEvent};

use crate::core::{
    aggregate::Aggregate, command_handler::CommandHandler, event_handler::EventHandler,
    event_store::StreamId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserId(pub usize);

impl From<UserId> for StreamId {
    fn from(id: UserId) -> Self {
        StreamId(id.0)
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "user-{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Customer,
    Agent,
    Admin,
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Customer => "Customer",
            Self::Agent => "Agent",
            Self::Admin => "Admin",
        };
        f.write_str(name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    user_id: UserId,
    user_name: String,
    role: Role,
}

impl User {
    /// Решение о регистрации нового пользователя.
    pub fn register(user_id: UserId, user_name: &str, role: Role) -> Result<UserEvent, UserError> {
        let user_name = user_name.trim();
        if user_name.is_empty() {
            return Err(UserError::EmptyName);
        }
        Ok(UserEvent::Registered(UserRegisteredEvent {
            user_id,
            user_name: user_name.to_owned(),
            role,
        }))
    }

    pub fn user_id(&self) -> UserId {
        self.user_id
    }

    pub fn user_name(&self) -> &str {
        &self.user_name
    }

    pub fn role(&self) -> Role {
        self.role
    }
}

impl From<&UserRegisteredEvent> for User {
    fn from(event: &UserRegisteredEvent) -> Self {
        Self {
            user_id: event.user_id,
            user_name: event.user_name.clone(),
            role: event.role,
        }
    }
}

/// Агрегат — «пользователь, которого может ещё не быть»: `None` соответствует пустому стриму.
impl Aggregate for Option<User> {
    type Event = UserEvent;
    type Command = UserCommand;
}

impl CommandHandler for Option<User> {
    type Error = UserError;

    fn decide(&self, command: &UserCommand) -> Result<UserEvent, UserError> {
        match (self, command) {
            (
                None,
                UserCommand::Register {
                    user_id,
                    user_name,
                    role,
                },
            ) => User::register(*user_id, user_name, *role),
            (Some(user), UserCommand::Register { .. }) => {
                Err(UserError::AlreadyExists(user.user_id))
            }
        }
    }
}

impl EventHandler for Option<User> {
    fn apply_event(self, event: &UserEvent) -> Self {
        match (self, event) {
            (None, UserEvent::Registered(registered)) => Some(User::from(registered)),
            (Some(user), UserEvent::Registered(_)) => {
                panic!("corrupted stream: {} registered twice", user.user_id)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::event_sourced::EventSourced;

    fn register(name: &str, role: Role) -> UserCommand {
        UserCommand::Register {
            user_id: UserId(1),
            user_name: name.to_owned(),
            role,
        }
    }

    #[test]
    fn register_on_empty_stream_emits_registered_event() {
        assert_eq!(
            None::<User>.decide(&register(" Alice ", Role::Agent)),
            Ok(UserEvent::Registered(UserRegisteredEvent {
                user_id: UserId(1),
                user_name: "Alice".to_owned(),
                role: Role::Agent,
            }))
        );
    }

    #[test]
    fn blank_name_is_rejected() {
        assert_eq!(
            None::<User>.decide(&register("  ", Role::Agent)),
            Err(UserError::EmptyName)
        );
    }

    #[test]
    fn user_cannot_be_registered_twice() {
        let event = None::<User>
            .decide(&register("Alice", Role::Agent))
            .unwrap();
        let user = None::<User>.recover([event]);
        assert_eq!(
            user.decide(&register("Bob", Role::Customer)),
            Err(UserError::AlreadyExists(UserId(1)))
        );
    }

    #[test]
    fn registered_event_builds_user_state() {
        let event = None::<User>
            .decide(&register("Alice", Role::Admin))
            .unwrap();
        let user = None::<User>.recover([event]).expect("user is registered");
        assert_eq!(user.user_id(), UserId(1));
        assert_eq!(user.user_name(), "Alice");
        assert_eq!(user.role(), Role::Admin);
    }
}
