use std::collections::HashMap;

use crate::domain::{
    tenant::TenantId,
    ticket::{TicketEvent, TicketId, inactivity_expired},
    time::Timestamp,
};

/// Read-модель (проекция) открытых тикетов, построенная только из событий тикетов.
///
/// Отвечает на вопросы, которые неудобно задавать отдельным агрегатам:
/// сколько открытых тикетов у тенанта и какие тикеты простаивают.
#[derive(Debug, Default)]
pub struct OpenTickets {
    tickets: HashMap<TicketId, OpenTicket>,
}

#[derive(Debug, Clone, Copy)]
struct OpenTicket {
    tenant_id: TenantId,
    last_activity: Timestamp,
}

impl OpenTickets {
    pub fn project(&mut self, event: &TicketEvent) {
        match event {
            TicketEvent::Created(e) => {
                self.tickets.insert(
                    e.ticket_id,
                    OpenTicket {
                        tenant_id: e.tenant_id,
                        last_activity: e.at,
                    },
                );
            }
            TicketEvent::AgentAssigned(e) => {
                if let Some(ticket) = self.tickets.get_mut(&e.ticket_id) {
                    ticket.last_activity = e.at;
                }
            }
            TicketEvent::Closed(e) => {
                self.tickets.remove(&e.ticket_id);
            }
        }
    }

    pub fn count_for_tenant(&self, tenant_id: TenantId) -> usize {
        self.tickets
            .values()
            .filter(|t| t.tenant_id == tenant_id)
            .count()
    }

    /// Кандидаты на закрытие по неактивности. Окончательно решает сам агрегат `Ticket`.
    pub fn inactive_at(&self, now: Timestamp) -> Vec<TicketId> {
        let mut ids: Vec<_> = self
            .tickets
            .iter()
            .filter(|(_, t)| inactivity_expired(t.last_activity, now))
            .map(|(id, _)| *id)
            .collect();
        ids.sort();
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        ticket::{
            AgentAssignedEvent, CloseReason, INACTIVITY_TIMEOUT_SECS, TicketClosedEvent,
            TicketCreatedEvent,
        },
        user::UserId,
    };

    fn created(ticket: usize, tenant: usize, at: u64) -> TicketEvent {
        TicketEvent::Created(TicketCreatedEvent {
            ticket_id: TicketId(ticket),
            category: "c".to_owned(),
            description: "d".to_owned(),
            tenant_id: TenantId(tenant),
            at: Timestamp(at),
        })
    }

    fn closed(ticket: usize) -> TicketEvent {
        TicketEvent::Closed(TicketClosedEvent {
            ticket_id: TicketId(ticket),
            reason: CloseReason::Resolved,
            at: Timestamp(0),
        })
    }

    #[test]
    fn counts_only_open_tickets_of_tenant() {
        let mut view = OpenTickets::default();
        for event in [
            created(1, 1, 0),
            created(2, 1, 0),
            created(3, 2, 0),
            closed(1),
        ] {
            view.project(&event);
        }
        assert_eq!(view.count_for_tenant(TenantId(1)), 1);
        assert_eq!(view.count_for_tenant(TenantId(2)), 1);
        assert_eq!(view.count_for_tenant(TenantId(3)), 0);
    }

    #[test]
    fn assignment_refreshes_activity() {
        let mut view = OpenTickets::default();
        view.project(&created(1, 1, 0));
        view.project(&created(2, 1, 0));
        view.project(&TicketEvent::AgentAssigned(AgentAssignedEvent {
            ticket_id: TicketId(2),
            agent_id: UserId(1),
            previous_agent_id: None,
            at: Timestamp(100),
        }));
        assert_eq!(
            view.inactive_at(Timestamp(INACTIVITY_TIMEOUT_SECS)),
            [TicketId(1)]
        );
        assert_eq!(
            view.inactive_at(Timestamp(INACTIVITY_TIMEOUT_SECS + 100)),
            [TicketId(1), TicketId(2)]
        );
    }

    #[test]
    fn closed_tickets_are_never_inactive() {
        let mut view = OpenTickets::default();
        view.project(&created(1, 1, 0));
        view.project(&closed(1));
        assert!(view.inactive_at(Timestamp(u64::MAX)).is_empty());
    }
}
