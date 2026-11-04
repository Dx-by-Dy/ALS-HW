use std::collections::HashMap;

use crate::domain::{
    tenant::TenantId,
    ticket::{TicketEvent, TicketId},
};

/// Проекция открытых тикетов, построенная только из событий тикетов.
///
/// Отвечает на вопрос, который неудобно задавать отдельным агрегатам:
/// сколько открытых тикетов у тенанта.
#[derive(Debug, Default)]
pub struct OpenTickets {
    tickets: HashMap<TicketId, TenantId>,
}

impl OpenTickets {
    pub fn project(&mut self, event: &TicketEvent) {
        match event {
            TicketEvent::Created(e) => {
                self.tickets.insert(e.ticket_id, e.tenant_id);
            }
            TicketEvent::Closed(e) => {
                self.tickets.remove(&e.ticket_id);
            }
            TicketEvent::Reopened(e) => {
                self.tickets.insert(e.ticket_id, e.tenant_id);
            }
            TicketEvent::AgentAssigned(_) | TicketEvent::ReminderSent(_) => {}
        }
    }

    pub fn count_for_tenant(&self, tenant_id: TenantId) -> usize {
        self.tickets
            .values()
            .filter(|tenant| **tenant == tenant_id)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        ticket::{
            AgentAssignedEvent, CloseReason, ReminderKind, ReminderSentEvent, TicketClosedEvent,
            TicketCreatedEvent, TicketReopenedEvent,
        },
        time::Timestamp,
        user::UserId,
    };

    fn created(ticket: usize, tenant: usize) -> TicketEvent {
        TicketEvent::Created(TicketCreatedEvent {
            ticket_id: TicketId(ticket),
            category: "c".to_owned(),
            description: "d".to_owned(),
            tenant_id: TenantId(tenant),
            at: Timestamp(0),
        })
    }

    fn closed(ticket: usize, tenant: usize) -> TicketEvent {
        TicketEvent::Closed(TicketClosedEvent {
            ticket_id: TicketId(ticket),
            tenant_id: TenantId(tenant),
            agent_id: None,
            reason: CloseReason::Resolved,
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
