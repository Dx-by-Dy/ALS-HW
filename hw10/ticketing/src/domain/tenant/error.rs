use std::error::Error;
use std::fmt;

use super::{DiscountRate, TenantId};

/// Нарушения инвариантов агрегата `Tenant`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TenantError {
    /// Тенант с таким идентификатором уже зарегистрирован.
    AlreadyExists(TenantId),
    /// Команда пришла в пустой стрим: тенант не зарегистрирован.
    NotFound,
    /// Название тенанта не может быть пустым.
    EmptyName,
    /// Политика даёт ту же скидку, что уже действует: менять нечего, события нет.
    DiscountUnchanged(DiscountRate),
}

impl fmt::Display for TenantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyExists(id) => write!(f, "{id} is already registered"),
            Self::NotFound => write!(f, "tenant does not exist"),
            Self::EmptyName => write!(f, "tenant name must not be empty"),
            Self::DiscountUnchanged(rate) => write!(f, "discount is already {rate}"),
        }
    }
}

impl Error for TenantError {}
