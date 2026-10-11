use std::error::Error;
use std::fmt;

use super::repository::RepositoryError;
use crate::domain::{
    tenant::{TenantError, TenantId},
    ticket::TicketError,
    user::{UserError, UserId},
};
use crate::infrastructure::error::StorageError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceError {
    Ticket(TicketError),
    Tenant(TenantError),
    User(UserError),
    /// Команда ссылается на незарегистрированного тенанта.
    UnknownTenant(TenantId),
    /// Команда ссылается на незарегистрированного пользователя.
    UnknownUser(UserId),
    Storage(StorageError),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ticket(e) => e.fmt(f),
            Self::Tenant(e) => e.fmt(f),
            Self::User(e) => e.fmt(f),
            Self::UnknownTenant(id) => write!(f, "{id} is not registered"),
            Self::UnknownUser(id) => write!(f, "{id} is not registered"),
            Self::Storage(e) => write!(f, "storage error: {e}"),
        }
    }
}

impl Error for ServiceError {}

impl From<TicketError> for ServiceError {
    fn from(e: TicketError) -> Self {
        Self::Ticket(e)
    }
}

impl From<TenantError> for ServiceError {
    fn from(e: TenantError) -> Self {
        Self::Tenant(e)
    }
}

impl From<UserError> for ServiceError {
    fn from(e: UserError) -> Self {
        Self::User(e)
    }
}

impl From<StorageError> for ServiceError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl<D> From<RepositoryError<D, StorageError>> for ServiceError
where
    ServiceError: From<D>,
{
    fn from(e: RepositoryError<D, StorageError>) -> Self {
        match e {
            RepositoryError::Domain(e) => e.into(),
            RepositoryError::Storage(e) => e.into(),
        }
    }
}
