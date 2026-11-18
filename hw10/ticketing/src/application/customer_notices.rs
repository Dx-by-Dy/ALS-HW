//! Шаг 4 бизнес-процесса — сообщить клиенту о его тикете через CRM.
//!
//! Клиент узнаёт, что обращение принято (и к какому сроку его обещают решить), что оно
//! закрыто (и уложились ли в срок) и что его открыли заново. Назначения агентов, напоминания
//! и работа с партнёрами — внутренняя кухня, о ней клиенту не пишут.

use framework::cqrs::{
    envelope::Version,
    event_source::{EventSource, Subscription},
    policy::Policy,
};

use super::ports::{CustomerNotice, CustomerNoticeKind, CustomerNotifier, ExternalError};
use crate::domain::{DomainEvent, ticket::TicketEvent};

/// Какое сообщение клиенту следует из события.
#[derive(Debug, Clone, Copy, Default)]
pub struct CustomerNoticePolicy;

impl Policy<DomainEvent> for CustomerNoticePolicy {
    type Command = CustomerNotice;

    fn react(&self, event: &DomainEvent) -> Vec<CustomerNotice> {
        let DomainEvent::Ticket(event) = event else {
            return Vec::new();
        };
        let (tenant_id, kind, at) = match event {
            TicketEvent::Created(e) => (
                e.tenant_id,
                CustomerNoticeKind::Received {
                    due_at: e.sla.due_at(),
                },
                e.at,
            ),
            TicketEvent::Closed(e) => (
                e.tenant_id,
                CustomerNoticeKind::Closed {
                    reason: e.reason,
                    sla_met: e.sla_met,
                },
                e.at,
            ),
            TicketEvent::Reopened(e) => (
                e.tenant_id,
                CustomerNoticeKind::Reopened {
                    due_at: e.sla.due_at(),
                },
                e.at,
            ),
            TicketEvent::AgentAssigned(_)
            | TicketEvent::ReminderSent(_)
            | TicketEvent::ResourcesChecked(_)
            | TicketEvent::HandedToPartner(_)
            | TicketEvent::PartnerProgressReported(_) => return Vec::new(),
        };
        vec![CustomerNotice {
            tenant_id,
            ticket_id: event.ticket_id(),
            kind,
            at,
        }]
    }
}

/// Отправляет клиентам сообщения по событиям с версией больше `after`, по одному и по порядку.
///
/// Неудачная отправка уходит в `on_error` и не повторяется: сообщение клиенту не должно
/// останавливать остальные. Завершается `Ok`, когда источник закрывает поток.
pub async fn run_customer_notices<S, N>(
    source: &S,
    after: Version,
    notifier: &N,
    mut on_error: impl FnMut(CustomerNotice, ExternalError),
) -> Result<(), S::Error>
where
    S: EventSource<DomainEvent>,
    N: CustomerNotifier,
{
    let mut events = source.subscribe(after).await?;
    while let Some(envelope) = events.next().await {
        for notice in CustomerNoticePolicy.react(&envelope.event) {
            if let Err(e) = notifier.notify_customer(&notice).await {
                on_error(notice, e);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::application::testing::Journal;
    use crate::domain::{
        partner::PartnerId,
        sla::{CustomerTier, Sla},
        tenant::TenantId,
        ticket::{
            AgentAssignedEvent, CloseReason, HandedToPartnerEvent, TicketClosedEvent,
            TicketCreatedEvent, TicketId, TicketReopenedEvent,
        },
        time::Timestamp,
        user::UserId,
    };

    const ACME: TenantId = TenantId(1);
    const TICKET: TicketId = TicketId(7);
    const AT: Timestamp = Timestamp(100);

    fn created() -> DomainEvent {
        TicketEvent::Created(TicketCreatedEvent {
            ticket_id: TICKET,
            category: "Hardware".to_owned(),
            description: "Disk".to_owned(),
            tenant_id: ACME,
            tier: CustomerTier::Gold,
            sla: Sla::for_tier(CustomerTier::Gold, AT),
            at: AT,
        })
        .into()
    }

    fn closed(reason: CloseReason, sla_met: bool) -> DomainEvent {
        TicketEvent::Closed(TicketClosedEvent {
            ticket_id: TICKET,
            tenant_id: ACME,
            agent_id: None,
            reason,
            sla_met,
            at: AT,
        })
        .into()
    }

    fn notice(kind: CustomerNoticeKind) -> CustomerNotice {
        CustomerNotice {
            tenant_id: ACME,
            ticket_id: TICKET,
            kind,
            at: AT,
        }
    }

    #[test]
    fn customer_learns_when_ticket_is_received_and_when_it_is_due() {
        assert_eq!(
            CustomerNoticePolicy.react(&created()),
            [notice(CustomerNoticeKind::Received {
                due_at: AT.plus_secs(CustomerTier::Gold.resolution_secs()),
            })]
        );
    }

    #[test]
    fn customer_learns_how_ticket_was_closed_and_whether_sla_was_met() {
        assert_eq!(
            CustomerNoticePolicy.react(&closed(CloseReason::Inactivity, false)),
            [notice(CustomerNoticeKind::Closed {
                reason: CloseReason::Inactivity,
                sla_met: false,
            })]
        );
    }

    #[test]
    fn customer_learns_about_reopening_with_new_deadline() {
        let reopened = TicketEvent::Reopened(TicketReopenedEvent {
            ticket_id: TICKET,
            tenant_id: ACME,
            category: "Hardware".to_owned(),
            sla: Sla::new(AT, 50),
            at: AT,
        })
        .into();
        assert_eq!(
            CustomerNoticePolicy.react(&reopened),
            [notice(CustomerNoticeKind::Reopened {
                due_at: Timestamp(150)
            })]
        );
    }

    #[test]
    fn internal_steps_are_not_told_to_customer() {
        let assigned = TicketEvent::AgentAssigned(AgentAssignedEvent {
            ticket_id: TICKET,
            agent_id: UserId(1),
            previous_agent_id: None,
            at: AT,
        });
        let handed = TicketEvent::HandedToPartner(HandedToPartnerEvent {
            ticket_id: TICKET,
            partner_id: PartnerId(1),
            at: AT,
        });
        for event in [assigned, handed] {
            assert_eq!(CustomerNoticePolicy.react(&event.into()), []);
        }
    }

    /// Запоминает сообщения; закрытие по неактивности CRM не принимает.
    #[derive(Default)]
    struct Recording(Mutex<Vec<CustomerNotice>>);

    impl CustomerNotifier for Recording {
        async fn notify_customer(&self, notice: &CustomerNotice) -> Result<(), ExternalError> {
            if matches!(
                notice.kind,
                CustomerNoticeKind::Closed {
                    reason: CloseReason::Inactivity,
                    ..
                }
            ) {
                return Err(ExternalError::Unavailable {
                    system: "CRM",
                    reason: "down".to_owned(),
                });
            }
            self.0.lock().unwrap().push(*notice);
            Ok(())
        }
    }

    #[tokio::test]
    async fn failed_notice_is_reported_and_others_go_on() {
        let source = Journal(vec![
            created(),
            closed(CloseReason::Inactivity, false),
            closed(CloseReason::Resolved, true),
        ]);
        let notifier = Recording::default();
        let mut failed = Vec::new();
        run_customer_notices(&source, Version(0), &notifier, |n, _| failed.push(n))
            .await
            .unwrap();
        assert_eq!(failed.len(), 1);
        let sent: Vec<_> = notifier.0.lock().unwrap().iter().map(|n| n.kind).collect();
        assert_eq!(sent.len(), 2);
        assert!(matches!(sent[0], CustomerNoticeKind::Received { .. }));
        assert!(matches!(
            sent[1],
            CustomerNoticeKind::Closed {
                reason: CloseReason::Resolved,
                sla_met: true
            }
        ));
    }
}
