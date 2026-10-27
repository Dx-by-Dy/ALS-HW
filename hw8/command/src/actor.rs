use ticketing::application::{clock::Clock, error::ServiceError, ticket_service::TicketService};
use ticketing::core::envelope::{Envelope, Version};
use ticketing::domain::DomainEvent;
use tokio::sync::mpsc;

use crate::bus::{CommandBus, Committed, EventReceiver, Message};
use crate::command::Command;

/// Сколько команд может ждать в очереди, прежде чем отправители начнут ждать места.
const INBOX_CAPACITY: usize = 64;

/// Запускает актор в задаче tokio и возвращает ручку к нему.
/// Актор останавливается, когда пропадает последний клон [`CommandBus`].
pub fn spawn<C>(service: TicketService<C>) -> CommandBus
where
    C: Clock + Send + Sync + 'static,
{
    let (sender, inbox) = mpsc::channel(INBOX_CAPACITY);
    let actor = CommandActor {
        service,
        inbox,
        journal: Vec::new(),
        subscribers: Vec::new(),
    };
    tokio::spawn(actor.run());
    CommandBus::new(sender)
}

/// Единственный писатель: команды выполняются по одной, поэтому между проверкой
/// и записью ничего не вклинивается, а у событий однозначный общий порядок.
struct CommandActor<C> {
    service: TicketService<C>,
    inbox: mpsc::Receiver<Message>,
    /// Общий журнал всех стримов в порядке записи: на него можно подписаться с любой версии.
    journal: Vec<Envelope<DomainEvent>>,
    subscribers: Vec<mpsc::UnboundedSender<Envelope<DomainEvent>>>,
}

impl<C: Clock> CommandActor<C> {
    async fn run(mut self) {
        while let Some(message) = self.inbox.recv().await {
            match message {
                Message::Execute { command, reply } => {
                    let result = self.execute(command).await;
                    // Отправитель мог не дождаться ответа — команда всё равно выполнена.
                    let _ = reply.send(result);
                }
                Message::Subscribe { after, reply } => {
                    let _ = reply.send(self.subscribe(after));
                }
            }
        }
    }

    async fn execute(&mut self, command: Command) -> Result<Committed, ServiceError> {
        let result = self.dispatch(command).await;
        // Публикуем и при ошибке: команда могла успеть записать часть событий.
        let events = self.publish_outbox();
        result.map(|()| Committed {
            version: self.version(),
            events,
        })
    }

    async fn dispatch(&mut self, command: Command) -> Result<(), ServiceError> {
        let service = &mut self.service;
        match command {
            Command::RegisterTenant(c) => service.register_tenant(c).await.map(drop),
            Command::RegisterUser(c) => service.register_user(c).await.map(drop),
            Command::CreateTicket(c) => service.handle_create_ticket(c).await.map(drop),
            Command::AssignAgent(c) => service.handle_assign_agent(c).await.map(drop),
            Command::CloseTicket(c) => service.handle_close_ticket(c).await.map(drop),
            Command::ReopenTicket(c) => service.handle_reopen_ticket(c).await.map(drop),
            Command::ApplyDiscount(c) => service.handle_apply_discount(c).await.map(drop),
            Command::CloseInactiveTickets => service.close_inactive_tickets().await.map(drop),
        }
    }

    /// Присваивает версии новым событиям, дописывает их в журнал и рассылает подписчикам.
    fn publish_outbox(&mut self) -> Vec<Envelope<DomainEvent>> {
        let mut published = Vec::new();
        for event in self.service.take_outbox() {
            let envelope = Envelope {
                version: self.version().next(),
                event,
            };
            // Подписчик, который закрыл свой конец канала, больше не нужен.
            self.subscribers
                .retain(|subscriber| subscriber.send(envelope.clone()).is_ok());
            self.journal.push(envelope.clone());
            published.push(envelope);
        }
        published
    }

    fn subscribe(&mut self, after: Version) -> EventReceiver {
        let (sender, receiver) = mpsc::unbounded_channel();
        let history = usize::try_from(after.0).unwrap_or(usize::MAX);
        for envelope in self.journal.iter().skip(history) {
            // Получатель ещё у нас в руках, отправка не может провалиться.
            let _ = sender.send(envelope.clone());
        }
        self.subscribers.push(sender);
        receiver
    }

    fn version(&self) -> Version {
        Version(self.journal.len() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CommandError;
    use ticketing::application::{
        clock::ManualClock,
        commands::{
            AssignAgentCommand, CloseTicketCommand, CreateTicketCommand, RegisterTenantCommand,
            RegisterUserCommand,
        },
    };
    use ticketing::domain::{
        tenant::TenantId,
        ticket::{CloseReason, INACTIVITY_TIMEOUT_SECS, TicketEvent, TicketId},
        user::{Role, UserId},
    };

    fn versions(committed: &Committed) -> Vec<u64> {
        committed.events.iter().map(|e| e.version.0).collect()
    }

    fn ticket(tenant_id: TenantId) -> CreateTicketCommand {
        CreateTicketCommand::new("Billing", "Wrong invoice", tenant_id)
    }

    /// Актор с тенантом `TenantId(1)` (версия 1) и агентом `UserId(1)` (версия 2).
    async fn bus_with_tenant() -> (CommandBus, ManualClock) {
        let clock = ManualClock::default();
        let bus = spawn(TicketService::new(clock.clone()));
        bus.send(RegisterTenantCommand::new("Acme")).await.unwrap();
        bus.send(RegisterUserCommand::new("Alice", Role::Agent))
            .await
            .unwrap();
        (bus, clock)
    }

    #[tokio::test]
    async fn every_event_gets_next_global_version() {
        let bus = spawn(TicketService::new(ManualClock::default()));
        let tenant = bus.send(RegisterTenantCommand::new("Acme")).await.unwrap();
        assert_eq!(tenant.version, Version(1));
        assert!(matches!(
            tenant.events[0].event,
            DomainEvent::Tenant(ref e) if e.tenant_id() == TenantId(1)
        ));

        let ticket = bus.send(ticket(TenantId(1))).await.unwrap();
        assert_eq!(ticket.version, Version(2));
        assert_eq!(versions(&ticket), [2]);
    }

    #[tokio::test]
    async fn rejected_command_takes_no_version() {
        let (bus, _) = bus_with_tenant().await;
        assert_eq!(
            bus.send(ticket(TenantId(99))).await,
            Err(CommandError::Rejected(ServiceError::UnknownTenant(
                TenantId(99)
            )))
        );
        let committed = bus.send(ticket(TenantId(1))).await.unwrap();
        assert_eq!(committed.version, Version(3));
    }

    #[tokio::test]
    async fn command_that_writes_nothing_reports_current_version() {
        let (bus, _) = bus_with_tenant().await;
        let committed = bus.send(Command::CloseInactiveTickets).await.unwrap();
        assert_eq!(
            committed,
            Committed {
                version: Version(2),
                events: vec![],
            }
        );
    }

    #[tokio::test]
    async fn closing_inactive_tickets_publishes_each_closed_ticket() {
        let (bus, clock) = bus_with_tenant().await;
        for _ in 0..2 {
            bus.send(ticket(TenantId(1))).await.unwrap();
        }
        clock.advance(INACTIVITY_TIMEOUT_SECS);

        let committed = bus.send(Command::CloseInactiveTickets).await.unwrap();
        assert_eq!(committed.version, Version(6));
        assert_eq!(versions(&committed), [5, 6]);
        assert!(committed.events.iter().all(|e| matches!(
            e.event,
            DomainEvent::Ticket(TicketEvent::Closed(ref closed))
                if closed.reason == CloseReason::Inactivity
        )));
    }

    #[tokio::test]
    async fn subscriber_gets_history_after_version_then_live_events() {
        let (bus, _) = bus_with_tenant().await;
        let mut events = bus.subscribe(Version(1)).await.unwrap();
        assert_eq!(events.recv().await.unwrap().version, Version(2));
        assert!(events.try_recv().is_err());

        bus.send(ticket(TenantId(1))).await.unwrap();
        bus.send(CloseTicketCommand {
            ticket_id: TicketId(1),
            reason: CloseReason::Resolved,
        })
        .await
        .unwrap();
        let live: Vec<_> = [events.recv().await, events.recv().await]
            .into_iter()
            .map(|e| e.unwrap().version)
            .collect();
        assert_eq!(live, [Version(3), Version(4)]);
    }

    #[tokio::test]
    async fn subscriber_after_last_version_gets_only_new_events() {
        let (bus, _) = bus_with_tenant().await;
        let mut events = bus.subscribe(Version(100)).await.unwrap();
        assert!(events.try_recv().is_err());
        bus.send(ticket(TenantId(1))).await.unwrap();
        assert_eq!(events.recv().await.unwrap().version, Version(3));
    }

    #[tokio::test]
    async fn rejected_command_reaches_no_subscriber() {
        let (bus, _) = bus_with_tenant().await;
        let mut events = bus.subscribe(Version(2)).await.unwrap();
        let assign_missing_ticket = AssignAgentCommand {
            ticket_id: TicketId(1),
            agent_id: UserId(1),
        };
        assert!(bus.send(assign_missing_ticket).await.is_err());
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn concurrent_senders_are_serialized() {
        let (bus, _) = bus_with_tenant().await;
        let senders: Vec<_> = (0..10)
            .map(|_| {
                let bus = bus.clone();
                tokio::spawn(async move { bus.send(ticket(TenantId(1))).await.unwrap() })
            })
            .collect();
        let mut versions = Vec::new();
        for sender in senders {
            versions.push(sender.await.unwrap().version.0);
        }
        versions.sort();
        assert_eq!(versions, (3..=12).collect::<Vec<_>>());
    }
}
