use framework::cqrs::policy::Policy;

use super::commands::ApplyDiscountCommand;
use crate::domain::{DomainEvent, ticket::TicketEvent};

/// Скидка тенанта зависит от числа его открытых тикетов, поэтому любое событие,
/// которое это число меняет, — повод пересчитать скидку.
///
/// Политика только предлагает пересчёт: сколько тикетов открыто и меняется ли скидка,
/// решает командная сторона. Если скидка осталась прежней, команда отклоняется
/// с `DiscountUnchanged`, и это нормальный исход.
#[derive(Debug, Clone, Copy, Default)]
pub struct DiscountPolicy;

impl Policy<DomainEvent> for DiscountPolicy {
    type Command = ApplyDiscountCommand;

    fn react(&self, event: &DomainEvent) -> Vec<ApplyDiscountCommand> {
        let tenant_id = match event {
            DomainEvent::Ticket(TicketEvent::Created(e)) => e.tenant_id,
            DomainEvent::Ticket(TicketEvent::Closed(e)) => e.tenant_id,
            DomainEvent::Ticket(TicketEvent::Reopened(e)) => e.tenant_id,
            DomainEvent::Ticket(TicketEvent::AgentAssigned(_))
            | DomainEvent::Tenant(_)
            | DomainEvent::User(_) => return Vec::new(),
        };
        vec![ApplyDiscountCommand { tenant_id }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        tenant::{DiscountAppliedEvent, DiscountRate, TenantEvent, TenantId},
        ticket::{
            AgentAssignedEvent, CloseReason, TicketClosedEvent, TicketCreatedEvent, TicketId,
            TicketReopenedEvent,
        },
        time::Timestamp,
        user::UserId,
    };

    const ACME: TenantId = TenantId(1);
    const TICKET: TicketId = TicketId(1);

    fn react(event: impl Into<DomainEvent>) -> Vec<ApplyDiscountCommand> {
        DiscountPolicy.react(&event.into())
    }

    #[test]
    fn events_changing_open_tickets_recalculate_discount() {
        let recalculate = vec![ApplyDiscountCommand { tenant_id: ACME }];
        let created = TicketEvent::Created(TicketCreatedEvent {
            ticket_id: TICKET,
            category: "Billing".to_owned(),
            description: "Wrong invoice".to_owned(),
            tenant_id: ACME,
            at: Timestamp(0),
        });
        let closed = TicketEvent::Closed(TicketClosedEvent {
            ticket_id: TICKET,
            tenant_id: ACME,
            reason: CloseReason::Inactivity,
            at: Timestamp(0),
        });
        let reopened = TicketEvent::Reopened(TicketReopenedEvent {
            ticket_id: TICKET,
            tenant_id: ACME,
            at: Timestamp(0),
        });
        assert_eq!(react(created), recalculate);
        assert_eq!(react(closed), recalculate);
        assert_eq!(react(reopened), recalculate);
    }

    #[test]
    fn other_events_are_ignored() {
        let assigned = TicketEvent::AgentAssigned(AgentAssignedEvent {
            ticket_id: TICKET,
            agent_id: UserId(1),
            previous_agent_id: None,
            at: Timestamp(0),
        });
        assert!(react(assigned).is_empty());

        // Иначе политика реагировала бы на собственный результат.
        let discount = TenantEvent::DiscountApplied(DiscountAppliedEvent {
            tenant_id: ACME,
            previous_rate: DiscountRate::NONE,
            discount_rate: DiscountRate::STANDARD,
            open_tickets: 6,
        });
        assert!(react(discount).is_empty());
    }
}
