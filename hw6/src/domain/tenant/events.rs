use std::fmt;

use super::{DiscountRate, TenantId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TenantEvent {
    Registered(TenantRegisteredEvent),
    DiscountApplied(DiscountAppliedEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantRegisteredEvent {
    pub tenant_id: TenantId,
    pub tenant_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscountAppliedEvent {
    pub tenant_id: TenantId,
    pub previous_rate: DiscountRate,
    pub discount_rate: DiscountRate,
    /// Сколько открытых тикетов было на момент решения.
    pub open_tickets: usize,
}

impl TenantEvent {
    pub fn tenant_id(&self) -> TenantId {
        match self {
            Self::Registered(e) => e.tenant_id,
            Self::DiscountApplied(e) => e.tenant_id,
        }
    }
}

impl fmt::Display for TenantEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registered(e) => {
                write!(f, "TenantRegistered: {} '{}'", e.tenant_id, e.tenant_name)
            }
            Self::DiscountApplied(e) => write!(
                f,
                "DiscountApplied: {} {} -> {} ({} open tickets)",
                e.tenant_id, e.previous_rate, e.discount_rate, e.open_tickets
            ),
        }
    }
}
