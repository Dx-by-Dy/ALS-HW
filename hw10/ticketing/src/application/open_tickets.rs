use std::collections::HashMap;

use crate::domain::{tenant::TenantId, ticket::TicketEvent};

/// Проекция открытых тикетов, построенная только из событий тикетов.
///
/// Отвечает на вопрос, который неудобно задавать отдельным агрегатам: сколько открытых тикетов
/// у тенанта. Хранит только счётчики, потому что спрашивают об этом при каждом пересчёте скидки,
/// а перебор всех тикетов рос бы вместе с системой. Каждое событие применяется один раз: их
/// порядок и единственность гарантирует запись в стрим, после которой вызывается `project`.
#[derive(Debug, Default)]
pub struct OpenTickets {
    by_tenant: HashMap<TenantId, usize>,
}

impl OpenTickets {
    pub fn project(&mut self, event: &TicketEvent) {
        match event {
            TicketEvent::Created(e) => self.opened(e.tenant_id),
            TicketEvent::Reopened(e) => self.opened(e.tenant_id),
            TicketEvent::Closed(e) => {
                if let Some(count) = self.by_tenant.get_mut(&e.tenant_id) {
                    *count = count.saturating_sub(1);
                }
            }
            TicketEvent::AgentAssigned(_)
            | TicketEvent::ReminderSent(_)
            | TicketEvent::ResourcesChecked(_)
            | TicketEvent::HandedToPartner(_)
            | TicketEvent::PartnerProgressReported(_) => {}
        }
    }

    pub fn count_for_tenant(&self, tenant_id: TenantId) -> usize {
        self.by_tenant.get(&tenant_id).copied().unwrap_or(0)
    }

    fn opened(&mut self, tenant_id: TenantId) {
        *self.by_tenant.entry(tenant_id).or_default() += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        ticket::{
            AgentAssignedEvent, CloseReason, ReminderKind, ReminderSentEvent, TicketClosedEvent,
            TicketCreatedEvent, TicketId, TicketReopenedEvent,
        },
        time::Timestamp,
        user::UserId,
    };
    use kernel::sla::{CustomerTier, Sla};

    fn created(ticket: usize, tenant: usize) -> TicketEvent {
        TicketEvent::Created(TicketCreatedEvent {
            ticket_id: TicketId(ticket),
            category: "c".to_owned(),
            description: "d".to_owned(),
            tenant_id: TenantId(tenant),
            tier: CustomerTier::Bronze,
            sla: Sla::for_tier(CustomerTier::Bronze, Timestamp(0)),
            at: Timestamp(0),
        })
    }

    fn closed(ticket: usize, tenant: usize) -> TicketEvent {
        TicketEvent::Closed(TicketClosedEvent {
            ticket_id: TicketId(ticket),
            tenant_id: TenantId(tenant),
            agent_id: None,
            reason: CloseReason::Resolved,
            sla_met: true,
            at: Timestamp(0),
        })
    }

    #[test]
    fn counts_only_open_tickets_of_tenant() {
        let mut view = OpenTickets::default();
        for event in [created(1, 1), created(2, 1), created(3, 2), closed(1, 1)] {
            view.project(&event);
        }
        assert_eq!(view.count_for_tenant(TenantId(1)), 1);
        assert_eq!(view.count_for_tenant(TenantId(2)), 1);
        assert_eq!(view.count_for_tenant(TenantId(3)), 0);
    }

    #[test]
    fn reopened_ticket_counts_again() {
        let mut view = OpenTickets::default();
        view.project(&created(1, 1));
        view.project(&closed(1, 1));
        assert_eq!(view.count_for_tenant(TenantId(1)), 0);
        view.project(&TicketEvent::Reopened(TicketReopenedEvent {
            ticket_id: TicketId(1),
            tenant_id: TenantId(1),
            sla: Sla::for_tier(CustomerTier::Bronze, Timestamp(0)),
            at: Timestamp(0),
        }));
        assert_eq!(view.count_for_tenant(TenantId(1)), 1);
    }

    #[test]
    fn assignment_and_reminder_do_not_change_the_count() {
        let mut view = OpenTickets::default();
        view.project(&created(1, 1));
        view.project(&TicketEvent::AgentAssigned(AgentAssignedEvent {
            ticket_id: TicketId(1),
            agent_id: UserId(1),
            previous_agent_id: None,
            at: Timestamp(0),
        }));
        view.project(&TicketEvent::ReminderSent(ReminderSentEvent {
            ticket_id: TicketId(1),
            agent_id: None,
            kind: ReminderKind::Stale,
            at: Timestamp(0),
        }));
        assert_eq!(view.count_for_tenant(TenantId(1)), 1);
    }
}
