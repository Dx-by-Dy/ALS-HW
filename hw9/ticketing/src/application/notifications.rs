use std::fmt;
use std::future::Future;

use framework::cqrs::{
    envelope::Version,
    event_source::{EventSource, Subscription},
    policy::Policy,
};

use crate::domain::{
    DomainEvent,
    ticket::{CloseReason, ReminderKind, TicketEvent, TicketId},
    time::Timestamp,
    user::UserId,
};

/// Кому адресовано уведомление.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Recipient {
    Agent(UserId),
    /// Все администраторы: так адресуется то, у чего нет ответственного агента.
    Admins,
}

impl fmt::Display for Recipient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Agent(id) => write!(f, "agent {id}"),
            Self::Admins => f.write_str("admins"),
        }
    }
}

/// О чём уведомление.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    /// Появился тикет, которому нужен агент.
    NewTicket,
    /// Тикет открыли заново, и ему снова нужен агент.
    Reopened,
    /// Тикет назначен на получателя.
    Assigned,
    /// Тикет передан другому агенту.
    Unassigned,
    Reminder(ReminderKind),
    Closed(CloseReason),
}

impl fmt::Display for NotificationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NewTicket => f.write_str("new ticket needs an agent"),
            Self::Reopened => f.write_str("reopened and needs an agent"),
            Self::Assigned => f.write_str("assigned to you"),
            Self::Unassigned => f.write_str("reassigned to another agent"),
            Self::Reminder(kind) => write!(f, "reminder: {kind}"),
            Self::Closed(reason) => write!(f, "closed: {reason}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Notification {
    pub recipient: Recipient,
    pub ticket_id: TicketId,
    pub kind: NotificationKind,
    pub at: Timestamp,
}

impl fmt::Display for Notification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "to {}: {} {} at {}",
            self.recipient, self.ticket_id, self.kind, self.at
        )
    }
}

/// Участник хореографии: решает, кого и о чём уведомить, глядя только на событие.
///
/// Ни о саге, ни о политике скидок он не знает, а они — о нём. Здесь чистая функция; отправляет
/// уведомления [`run_notifications`], поэтому те же правила может использовать и read-модель.
#[derive(Debug, Clone, Copy, Default)]
pub struct NotificationPolicy;

impl Policy<DomainEvent> for NotificationPolicy {
    type Command = Notification;

    fn react(&self, event: &DomainEvent) -> Vec<Notification> {
        let DomainEvent::Ticket(event) = event else {
            return Vec::new();
        };
        let ticket_id = event.ticket_id();
        let notify = |recipient, kind, at| Notification {
            recipient,
            ticket_id,
            kind,
            at,
        };
        match event {
            TicketEvent::Created(e) => {
                vec![notify(Recipient::Admins, NotificationKind::NewTicket, e.at)]
            }
            TicketEvent::Reopened(e) => {
                vec![notify(Recipient::Admins, NotificationKind::Reopened, e.at)]
            }
            TicketEvent::AgentAssigned(e) => {
                let mut notifications = vec![notify(
                    Recipient::Agent(e.agent_id),
                    NotificationKind::Assigned,
                    e.at,
                )];
                if let Some(previous) = e.previous_agent_id {
                    notifications.push(notify(
                        Recipient::Agent(previous),
                        NotificationKind::Unassigned,
                        e.at,
                    ));
                }
                notifications
            }
            TicketEvent::ReminderSent(e) => {
                let recipient = e.agent_id.map_or(Recipient::Admins, Recipient::Agent);
                vec![notify(recipient, NotificationKind::Reminder(e.kind), e.at)]
            }
            TicketEvent::Closed(e) => {
                let recipient = match (e.agent_id, e.reason) {
                    (Some(agent), _) => Recipient::Agent(agent),
                    // Администраторам напоминали об этом тикете: пусть узнают, чем всё кончилось.
                    (None, CloseReason::Inactivity) => Recipient::Admins,
                    (None, CloseReason::Resolved) => return Vec::new(),
                };
                vec![notify(recipient, NotificationKind::Closed(e.reason), e.at)]
            }
        }
    }
}

/// Порт доставки уведомлений наружу: почта, мессенджер, журнал.
pub trait Notifier {
    type Error;

    fn send(
        &self,
        notification: &Notification,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// Доставляет уведомления по событиям с версией больше `after`, по одному и по порядку.
///
/// Неудачная доставка уходит в `on_error` и не повторяется, а цикл идёт дальше: уведомление
/// не должно останавливать остальные. Завершается `Ok`, когда источник закрывает поток.
pub async fn run_notifications<S, N>(
    source: &S,
    after: Version,
    notifier: &N,
    mut on_error: impl FnMut(Notification, N::Error),
) -> Result<(), S::Error>
where
    S: EventSource<DomainEvent>,
    N: Notifier,
{
    let mut events = source.subscribe(after).await?;
    while let Some(envelope) = events.next().await {
        for notification in NotificationPolicy.react(&envelope.event) {
            if let Err(e) = notifier.send(&notification).await {
                on_error(notification, e);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use framework::cqrs::envelope::Envelope;

    use super::*;
    use crate::domain::{
        tenant::{TenantEvent, TenantId, TenantRegisteredEvent},
        ticket::{
            AgentAssignedEvent, ReminderSentEvent, TicketClosedEvent, TicketCreatedEvent,
            TicketReopenedEvent,
        },
    };

    const TICKET: TicketId = TicketId(7);
    const ALICE: UserId = UserId(1);
    const CAROL: UserId = UserId(2);
    const AT: Timestamp = Timestamp(100);

    fn notification(recipient: Recipient, kind: NotificationKind) -> Notification {
        Notification {
            recipient,
            ticket_id: TICKET,
            kind,
            at: AT,
        }
    }

    fn react(event: TicketEvent) -> Vec<Notification> {
        NotificationPolicy.react(&event.into())
    }

    fn created() -> TicketEvent {
        TicketEvent::Created(TicketCreatedEvent {
            ticket_id: TICKET,
            category: "Billing".to_owned(),
            description: "Invoice".to_owned(),
            tenant_id: TenantId(1),
            at: AT,
        })
    }

    fn assigned(previous_agent_id: Option<UserId>) -> TicketEvent {
        TicketEvent::AgentAssigned(AgentAssignedEvent {
            ticket_id: TICKET,
            agent_id: ALICE,
            previous_agent_id,
            at: AT,
        })
    }

    fn closed(agent_id: Option<UserId>, reason: CloseReason) -> TicketEvent {
        TicketEvent::Closed(TicketClosedEvent {
            ticket_id: TICKET,
            tenant_id: TenantId(1),
            agent_id,
            reason,
            at: AT,
        })
    }

    fn reminder(agent_id: Option<UserId>) -> TicketEvent {
        TicketEvent::ReminderSent(ReminderSentEvent {
            ticket_id: TICKET,
            agent_id,
            kind: ReminderKind::Stale,
            at: AT,
        })
    }

    #[test]
    fn new_and_reopened_tickets_are_announced_to_admins() {
        assert_eq!(
            react(created()),
            [notification(Recipient::Admins, NotificationKind::NewTicket)]
        );
        let reopened = TicketEvent::Reopened(TicketReopenedEvent {
            ticket_id: TICKET,
            tenant_id: TenantId(1),
            at: AT,
        });
        assert_eq!(
            react(reopened),
            [notification(Recipient::Admins, NotificationKind::Reopened)]
        );
    }

    #[test]
    fn assignment_notifies_the_agent() {
        assert_eq!(
            react(assigned(None)),
            [notification(
                Recipient::Agent(ALICE),
                NotificationKind::Assigned
            )]
        );
    }

    #[test]
    fn reassignment_also_tells_the_previous_agent() {
        assert_eq!(
            react(assigned(Some(CAROL))),
            [
                notification(Recipient::Agent(ALICE), NotificationKind::Assigned),
                notification(Recipient::Agent(CAROL), NotificationKind::Unassigned),
            ]
        );
    }

    #[test]
    fn reminder_goes_to_the_agent_or_to_admins() {
        let kind = NotificationKind::Reminder(ReminderKind::Stale);
        assert_eq!(
            react(reminder(Some(ALICE))),
            [notification(Recipient::Agent(ALICE), kind)]
        );
        assert_eq!(
            react(reminder(None)),
            [notification(Recipient::Admins, kind)]
        );
    }

    #[test]
    fn closing_tells_whoever_is_responsible() {
        let inactivity = NotificationKind::Closed(CloseReason::Inactivity);
        assert_eq!(
            react(closed(Some(ALICE), CloseReason::Inactivity)),
            [notification(Recipient::Agent(ALICE), inactivity)]
        );
        assert_eq!(
            react(closed(None, CloseReason::Inactivity)),
            [notification(Recipient::Admins, inactivity)]
        );
        assert_eq!(
            react(closed(Some(ALICE), CloseReason::Resolved)),
            [notification(
                Recipient::Agent(ALICE),
                NotificationKind::Closed(CloseReason::Resolved)
            )]
        );
    }

    #[test]
    fn resolving_a_ticket_nobody_led_needs_no_notification() {
        assert_eq!(react(closed(None, CloseReason::Resolved)), []);
    }

    #[test]
    fn other_aggregates_are_ignored() {
        let tenant = TenantEvent::Registered(TenantRegisteredEvent {
            tenant_id: TenantId(1),
            tenant_name: "Acme".to_owned(),
        });
        assert_eq!(NotificationPolicy.react(&tenant.into()), []);
    }

    #[test]
    fn notification_is_readable_in_logs() {
        let reminder = notification(
            Recipient::Agent(ALICE),
            NotificationKind::Reminder(ReminderKind::ClosingSoon),
        );
        assert_eq!(
            reminder.to_string(),
            format!("to agent {ALICE}: ticket-7 reminder: closing soon at t=100s")
        );
        let closed = notification(
            Recipient::Admins,
            NotificationKind::Closed(CloseReason::Inactivity),
        );
        assert_eq!(
            closed.to_string(),
            "to admins: ticket-7 closed: inactivity at t=100s"
        );
    }

    /// Источник с заранее записанным журналом; поток закрывается, когда журнал кончился.
    struct Journal(Vec<DomainEvent>);

    struct Replay(VecDeque<Envelope<DomainEvent>>);

    impl Subscription<DomainEvent> for Replay {
        async fn next(&mut self) -> Option<Envelope<DomainEvent>> {
            self.0.pop_front()
        }
    }

    impl EventSource<DomainEvent> for Journal {
        type Error = &'static str;
        type Subscription = Replay;

        async fn subscribe(&self, after: Version) -> Result<Replay, &'static str> {
            let events = (1..)
                .map(Version)
                .zip(self.0.iter().cloned())
                .filter(|(version, _)| *version > after)
                .map(|(version, event)| Envelope { version, event })
                .collect();
            Ok(Replay(events))
        }
    }

    /// Запоминает уведомления; те, что адресованы Carol, доставить не может.
    #[derive(Default)]
    struct Recording(Mutex<Vec<Notification>>);

    impl Notifier for Recording {
        type Error = &'static str;

        async fn send(&self, notification: &Notification) -> Result<(), &'static str> {
            if notification.recipient == Recipient::Agent(CAROL) {
                return Err("mailbox is full");
            }
            self.0.lock().unwrap().push(*notification);
            Ok(())
        }
    }

    #[tokio::test]
    async fn notifications_are_sent_in_event_order() {
        let source = Journal(vec![created().into(), assigned(None).into()]);
        let notifier = Recording::default();
        let result = run_notifications(&source, Version(0), &notifier, |_, _| {
            panic!("everything is deliverable")
        })
        .await;
        assert_eq!(result, Ok(()));
        assert_eq!(
            *notifier.0.lock().unwrap(),
            [
                notification(Recipient::Admins, NotificationKind::NewTicket),
                notification(Recipient::Agent(ALICE), NotificationKind::Assigned),
            ]
        );
    }

    #[tokio::test]
    async fn only_events_after_version_are_delivered() {
        let source = Journal(vec![created().into(), assigned(None).into()]);
        let notifier = Recording::default();
        run_notifications(&source, Version(1), &notifier, |_, _| {})
            .await
            .unwrap();
        assert_eq!(
            *notifier.0.lock().unwrap(),
            [notification(
                Recipient::Agent(ALICE),
                NotificationKind::Assigned
            )]
        );
    }

    #[tokio::test]
    async fn failed_delivery_is_reported_and_others_go_on() {
        // При переназначении Carol не получит своё, но следующее событие всё равно обработается.
        let source = Journal(vec![assigned(Some(CAROL)).into(), created().into()]);
        let notifier = Recording::default();
        let mut failed = Vec::new();
        run_notifications(&source, Version(0), &notifier, |n, e| failed.push((n, e)))
            .await
            .unwrap();
        assert_eq!(
            failed,
            [(
                notification(Recipient::Agent(CAROL), NotificationKind::Unassigned),
                "mailbox is full"
            )]
        );
        assert_eq!(notifier.0.lock().unwrap().len(), 2);
    }
}
