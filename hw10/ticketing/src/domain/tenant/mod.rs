mod command;
mod discount;
mod error;
mod events;

use std::fmt;

pub use command::TenantCommand;
pub use discount::DiscountRate;
pub use error::TenantError;
pub use events::{DiscountAppliedEvent, TenantEvent, TenantRegisteredEvent};

use framework::event_sourcing::{event_store::StreamId, lifecycle::Lifecycle};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TenantId(pub usize);

impl From<TenantId> for StreamId {
    fn from(id: TenantId) -> Self {
        StreamId(id.0)
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tenant-{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tenant {
    tenant_id: TenantId,
    tenant_name: String,
    discount_rate: DiscountRate,
}

impl Tenant {
    /// Решение о регистрации нового тенанта.
    pub fn register(tenant_id: TenantId, tenant_name: &str) -> Result<TenantEvent, TenantError> {
        let tenant_name = tenant_name.trim();
        if tenant_name.is_empty() {
            return Err(TenantError::EmptyName);
        }
        Ok(TenantEvent::Registered(TenantRegisteredEvent {
            tenant_id,
            tenant_name: tenant_name.to_owned(),
        }))
    }

    /// Решение о пересчёте скидки по числу открытых тикетов (см. [`DiscountRate::for_open_tickets`]).
    pub fn apply_discount(&self, open_tickets: usize) -> Result<TenantEvent, TenantError> {
        let discount_rate = DiscountRate::for_open_tickets(open_tickets);
        if discount_rate == self.discount_rate {
            return Err(TenantError::DiscountUnchanged(discount_rate));
        }
        Ok(TenantEvent::DiscountApplied(DiscountAppliedEvent {
            tenant_id: self.tenant_id,
            previous_rate: self.discount_rate,
            discount_rate,
            open_tickets,
        }))
    }

    pub fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    pub fn tenant_name(&self) -> &str {
        &self.tenant_name
    }

    pub fn discount_rate(&self) -> DiscountRate {
        self.discount_rate
    }
}

impl From<&TenantRegisteredEvent> for Tenant {
    fn from(event: &TenantRegisteredEvent) -> Self {
        Self {
            tenant_id: event.tenant_id,
            tenant_name: event.tenant_name.clone(),
            discount_rate: DiscountRate::NONE,
        }
    }
}

/// Тенанта может ещё не быть: агрегат — `Option<Tenant>`, где `None` соответствует пустому стриму.
impl Lifecycle for Tenant {
    type Event = TenantEvent;
    type Command = TenantCommand;
    type Error = TenantError;

    fn decide_new(command: &TenantCommand) -> Result<TenantEvent, TenantError> {
        match command {
            TenantCommand::Register {
                tenant_id,
                tenant_name,
            } => Tenant::register(*tenant_id, tenant_name),
            TenantCommand::ApplyDiscount { .. } => Err(TenantError::NotFound),
        }
    }

    fn decide(&self, command: &TenantCommand) -> Result<TenantEvent, TenantError> {
        match command {
            TenantCommand::Register { .. } => Err(TenantError::AlreadyExists(self.tenant_id)),
            TenantCommand::ApplyDiscount { open_tickets } => self.apply_discount(*open_tickets),
        }
    }

    fn created(event: &TenantEvent) -> Self {
        match event {
            TenantEvent::Registered(registered) => Tenant::from(registered),
            other => panic!("corrupted stream: {other} before registration"),
        }
    }

    fn evolve(self, event: &TenantEvent) -> Self {
        match event {
            TenantEvent::Registered(_) => {
                panic!("corrupted stream: {} registered twice", self.tenant_id)
            }
            TenantEvent::DiscountApplied(e) => Self {
                discount_rate: e.discount_rate,
                ..self
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use framework::event_sourcing::{
        command_handler::CommandHandler, event_handler::EventHandler, event_sourced::EventSourced,
    };

    fn registered() -> Option<Tenant> {
        let command = TenantCommand::Register {
            tenant_id: TenantId(1),
            tenant_name: "Acme".to_owned(),
        };
        let event = None::<Tenant>.decide(&command).unwrap();
        None::<Tenant>.recover([event])
    }

    fn apply_discount(tenant: Option<Tenant>, open_tickets: usize) -> Option<Tenant> {
        let event = tenant
            .decide(&TenantCommand::ApplyDiscount { open_tickets })
            .unwrap();
        tenant.apply_event(&event)
    }

    #[test]
    fn registered_tenant_has_no_discount() {
        let tenant = registered().expect("tenant is registered");
        assert_eq!(tenant.tenant_id(), TenantId(1));
        assert_eq!(tenant.tenant_name(), "Acme");
        assert_eq!(tenant.discount_rate(), DiscountRate::NONE);
    }

    #[test]
    fn blank_name_is_rejected() {
        let command = TenantCommand::Register {
            tenant_id: TenantId(1),
            tenant_name: " ".to_owned(),
        };
        assert_eq!(None::<Tenant>.decide(&command), Err(TenantError::EmptyName));
    }

    #[test]
    fn tenant_cannot_be_registered_twice() {
        let command = TenantCommand::Register {
            tenant_id: TenantId(1),
            tenant_name: "Other".to_owned(),
        };
        assert_eq!(
            registered().decide(&command),
            Err(TenantError::AlreadyExists(TenantId(1)))
        );
    }

    #[test]
    fn discount_requires_registered_tenant() {
        assert_eq!(
            None::<Tenant>.decide(&TenantCommand::ApplyDiscount { open_tickets: 5 }),
            Err(TenantError::NotFound)
        );
    }

    #[test]
    fn apply_discount_emits_event_with_previous_and_new_rate() {
        assert_eq!(
            registered().decide(&TenantCommand::ApplyDiscount { open_tickets: 6 }),
            Ok(TenantEvent::DiscountApplied(DiscountAppliedEvent {
                tenant_id: TenantId(1),
                previous_rate: DiscountRate::NONE,
                discount_rate: DiscountRate::STANDARD,
                open_tickets: 6,
            }))
        );
    }

    #[test]
    fn discount_applied_event_changes_rate() {
        let tenant = apply_discount(registered(), 7).unwrap();
        assert_eq!(tenant.discount_rate(), DiscountRate::STANDARD);
    }

    #[test]
    fn discount_can_go_back_down() {
        let tenant = apply_discount(apply_discount(registered(), 12), 0).unwrap();
        assert_eq!(tenant.discount_rate(), DiscountRate::NONE);
    }

    #[test]
    fn up_to_threshold_tenant_keeps_no_discount() {
        assert_eq!(
            registered().decide(&TenantCommand::ApplyDiscount {
                open_tickets: DiscountRate::THRESHOLD
            }),
            Err(TenantError::DiscountUnchanged(DiscountRate::NONE))
        );
    }

    #[test]
    fn unchanged_discount_produces_no_event() {
        let tenant = apply_discount(registered(), 6);
        assert_eq!(
            tenant.decide(&TenantCommand::ApplyDiscount { open_tickets: 9 }),
            Err(TenantError::DiscountUnchanged(DiscountRate::STANDARD))
        );
    }
}
