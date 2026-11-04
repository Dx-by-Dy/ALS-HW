use std::error::Error;
use std::fmt;

use super::UserId;

/// Нарушения инвариантов агрегата `User`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserError {
    /// Пользователь с таким идентификатором уже зарегистрирован.
    AlreadyExists(UserId),
    /// Имя пользователя не может быть пустым.
    EmptyName,
}

impl fmt::Display for UserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyExists(id) => write!(f, "{id} is already registered"),
            Self::EmptyName => write!(f, "user name must not be empty"),
        }
    }
}

impl Error for UserError {}
