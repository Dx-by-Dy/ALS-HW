use framework::saga::{Saga, SagaEffects, effects::Effects};

use super::commands::{CloseTicketCommand, SendReminderCommand};
use crate::domain::{
    DomainEvent,
    ticket::{CloseReason, INACTIVITY_TIMEOUT_SECS, ReminderKind, TicketEvent, TicketId},
    time::Timestamp,
};

/// Дедлайны тикета. Срок каждого отсчитывается от последней активности.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TicketDeadline {
    Reminder(ReminderKind),
    AutoClose,
}

/// Команды, которые отдаёт сага.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleCommand {
    SendReminder(SendReminderCommand),
    CloseTicket(CloseTicketCommand),
}

/// Оркестратор жизненного цикла тикета: от создания до закрытия.
///
/// Запускается на `TicketCreated` и `TicketReopened`, завершается на `TicketClosed`, а вместе
/// с ним удаляются и дедлайны. Назначение агента и отчёт партнёра — активность, поэтому
/// переставляют все сроки.
///
/// Сроки считаются из времени в событиях, а не из текущего, поэтому состояние саги однозначно
/// восстанавливается из журнала. Команды может отклонить агрегат: дедлайн мог устареть,
/// пока команда шла к нему. Тогда о случившемся сообщит событие, которое её опередило:
/// `AgentAssigned` переставит сроки, `TicketClosed` завершит процесс. Поэтому и команда
/// автозакрытия процесс не завершает: сага дожидается самого `TicketClosed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TicketLifecycleSaga {
    ticket_id: TicketId,
}

/// Все сроки тикета, последняя активность которого случилась в момент `at`.
fn deadlines_from(at: Timestamp) -> SagaEffects<TicketLifecycleSaga> {
    let schedule_reminder = |effects: SagaEffects<TicketLifecycleSaga>, kind: ReminderKind| {
        effects.schedule(
            TicketDeadline::Reminder(kind),
            at.plus_secs(kind.due_after_secs()),
        )
    };
    let reminders = [ReminderKind::Stale, ReminderKind::ClosingSoon]
        .into_iter()
        .fold(Effects::none(), schedule_reminder);
    reminders.schedule(
        TicketDeadline::AutoClose,
        at.plus_secs(INACTIVITY_TIMEOUT_SECS),
    )
}

impl Saga for TicketLifecycleSaga {
    type Event = DomainEvent;
    type Command = LifecycleCommand;
    type Id = TicketId;
    type Deadline = TicketDeadline;
    type Time = Timestamp;

    fn id_of(event: &DomainEvent) -> Option<TicketId> {
        match event {
            DomainEvent::Ticket(e) => Some(e.ticket_id()),
            DomainEvent::Tenant(_) | DomainEvent::User(_) => None,
        }
    }

    fn start(event: &DomainEvent) -> Option<(Self, SagaEffects<Self>)> {
        let (ticket_id, at) = match event {
            DomainEvent::Ticket(TicketEvent::Created(e)) => (e.ticket_id, e.at),
            DomainEvent::Ticket(TicketEvent::Reopened(e)) => (e.ticket_id, e.at),
            _ => return None,
        };
        Some((Self { ticket_id }, deadlines_from(at)))
    }

    fn on_event(&mut self, event: &DomainEvent) -> SagaEffects<Self> {
        match event {
            DomainEvent::Ticket(TicketEvent::AgentAssigned(e)) => deadlines_from(e.at),
            DomainEvent::Ticket(TicketEvent::PartnerProgressReported(e)) => deadlines_from(e.at),
            DomainEvent::Ticket(TicketEvent::Reopened(e)) => deadlines_from(e.at),
            DomainEvent::Ticket(TicketEvent::Closed(_)) => Effects::none().finish(),
            // Напоминание, проверка ресурсов и передача партнёру — не активность,
            // и сроков они не меняют.
            DomainEvent::Ticket(
                TicketEvent::Created(_)
                | TicketEvent::ReminderSent(_)
                | TicketEvent::ResourcesChecked(_)
                | TicketEvent::HandedToPartner(_),
            )
            | DomainEvent::Tenant(_)
            | DomainEvent::User(_) => Effects::none(),
        }
    }

    fn on_deadline(&mut self, deadline: TicketDeadline) -> SagaEffects<Self> {
        let ticket_id = self.ticket_id;
        let command = match deadline {
            TicketDeadline::Reminder(kind) => {
                LifecycleCommand::SendReminder(SendReminderCommand { ticket_id, kind })
            }
            TicketDeadline::AutoClose => LifecycleCommand::CloseTicket(CloseTicketCommand {
                ticket_id,
                reason: CloseReason::Inactivity,
            }),
        };
        Effects::none().command(command)
    }
}

#[cfg(test)]
mod tests {
    use framework::saga::manager::SagaManager;

    use super::*;
    use crate::application::{
        clock::{Clock, ManualClock},
        commands::{
            AssignAgentCommand, CreateTicketCommand, RegisterTenantCommand, RegisterUserCommand,
            ReopenTicketCommand,
        },
        error::ServiceError,
        ticket_service::TicketService,
    };
    use crate::domain::{
        partner::PartnerId,
        tenant::{TenantEvent, TenantId, TenantRegisteredEvent},
        ticket::{
            AgentAssignedEvent, HandedToPartnerEvent, PartnerProgressReportedEvent,
            ReminderSentEvent, ResourceAvailability, ResourcesCheckedEvent, TicketClosedEvent,
            TicketCreatedEvent, TicketError, TicketReopenedEvent, TicketStatus,
        },
        user::{Role, UserId},
    };
    use kernel::sla::{CustomerTier, Sla};

    const DAY: u64 = 24 * 60 * 60;
    const T1: TicketId = TicketId(1);
    const T2: TicketId = TicketId(2);
    const STALE: TicketDeadline = TicketDeadline::Reminder(ReminderKind::Stale);
    const CLOSING_SOON: TicketDeadline = TicketDeadline::Reminder(ReminderKind::ClosingSoon);

    type Manager = SagaManager<TicketLifecycleSaga>;

    fn day(n: u64) -> Timestamp {
        Timestamp(n * DAY)
    }

    fn created(ticket_id: TicketId, at: Timestamp) -> DomainEvent {
        TicketEvent::Created(TicketCreatedEvent {
            ticket_id,
            category: "Billing".to_owned(),
            description: "Wrong invoice".to_owned(),
            tenant_id: TenantId(1),
            tier: CustomerTier::Bronze,
            sla: Sla::for_tier(CustomerTier::Bronze, at),
            at,
        })
        .into()
    }

    fn assigned(ticket_id: TicketId, at: Timestamp) -> DomainEvent {
        TicketEvent::AgentAssigned(AgentAssignedEvent {
            ticket_id,
            agent_id: UserId(1),
            previous_agent_id: None,
            at,
        })
        .into()
    }

    fn closed(ticket_id: TicketId, at: Timestamp) -> DomainEvent {
        TicketEvent::Closed(TicketClosedEvent {
            ticket_id,
            tenant_id: TenantId(1),
            agent_id: None,
            reason: CloseReason::Resolved,
            sla_met: true,
            at,
        })
        .into()
    }

    fn reopened(ticket_id: TicketId, at: Timestamp) -> DomainEvent {
        TicketEvent::Reopened(TicketReopenedEvent {
            ticket_id,
            tenant_id: TenantId(1),
            category: "Technical Issue".to_owned(),
            sla: Sla::for_tier(CustomerTier::Bronze, at),
            at,
        })
        .into()
    }

    fn reminder_sent(ticket_id: TicketId, kind: ReminderKind, at: Timestamp) -> DomainEvent {
        TicketEvent::ReminderSent(ReminderSentEvent {
            ticket_id,
            agent_id: None,
            kind,
            at,
        })
        .into()
    }

    fn remind(ticket_id: TicketId, kind: ReminderKind) -> LifecycleCommand {
        LifecycleCommand::SendReminder(SendReminderCommand { ticket_id, kind })
    }

    fn auto_close(ticket_id: TicketId) -> LifecycleCommand {
        LifecycleCommand::CloseTicket(CloseTicketCommand {
            ticket_id,
            reason: CloseReason::Inactivity,
        })
    }

    /// Всё, что наступило к `now`.
    fn fire_due(manager: &mut Manager, now: &Timestamp) -> Vec<LifecycleCommand> {
        let mut commands = Vec::new();
        while let Some(fired) = manager.fire_next(now) {
            commands.extend(fired);
        }
        commands
    }

    fn manager_with(events: impl IntoIterator<Item = DomainEvent>) -> Manager {
        let mut manager = Manager::default();
        for event in events {
            assert_eq!(manager.on_event(&event), []);
        }
        manager
    }

    #[test]
    fn creation_schedules_reminders_and_auto_close() {
        let manager = manager_with([created(T1, day(1))]);
        assert!(manager.is_running(&T1));
        assert_eq!(
            manager.deadlines_of(&T1),
            [
                (STALE, day(4)),
                (CLOSING_SOON, day(10)),
                (TicketDeadline::AutoClose, day(11)),
            ]
        );
    }

    #[test]
    fn closing_before_deadline_finishes_process_and_removes_deadlines() {
        let mut manager = manager_with([created(T1, day(0)), closed(T1, day(2))]);
        assert!(!manager.is_running(&T1));
        assert_eq!(manager.deadlines_of(&T1), []);
        assert_eq!(manager.next_deadline(), None);
        assert_eq!(fire_due(&mut manager, &day(100)), []);
    }

    #[test]
    fn deadlines_fire_in_their_time_and_not_earlier() {
        let mut manager = manager_with([created(T1, day(0))]);
        let before = |d: u64| Timestamp(d * DAY - 1);

        assert_eq!(fire_due(&mut manager, &before(3)), []);
        assert_eq!(
            fire_due(&mut manager, &day(3)),
            [remind(T1, ReminderKind::Stale)]
        );
        assert_eq!(fire_due(&mut manager, &before(9)), []);
        assert_eq!(
            fire_due(&mut manager, &day(9)),
            [remind(T1, ReminderKind::ClosingSoon)]
        );
        assert_eq!(fire_due(&mut manager, &before(10)), []);
        assert_eq!(fire_due(&mut manager, &day(10)), [auto_close(T1)]);
        assert_eq!(fire_due(&mut manager, &day(100)), []);
    }

    #[test]
    fn overdue_deadlines_are_sent_in_order() {
        let mut manager = manager_with([created(T1, day(0))]);
        assert_eq!(
            fire_due(&mut manager, &day(50)),
            [
                remind(T1, ReminderKind::Stale),
                remind(T1, ReminderKind::ClosingSoon),
                auto_close(T1),
            ]
        );
    }

    #[test]
    fn assignment_moves_all_deadlines() {
        let mut manager = manager_with([created(T1, day(0)), assigned(T1, day(2))]);
        assert_eq!(
            manager.deadlines_of(&T1),
            [
                (STALE, day(5)),
                (CLOSING_SOON, day(11)),
                (TicketDeadline::AutoClose, day(12)),
            ]
        );
        assert_eq!(fire_due(&mut manager, &day(4)), []);
    }

    #[test]
    fn assignment_after_reminder_brings_it_back() {
        let mut manager = manager_with([created(T1, day(0))]);
        fire_due(&mut manager, &day(3));
        assert_eq!(manager.deadlines_of(&T1).len(), 2);

        assert_eq!(manager.on_event(&assigned(T1, day(4))), []);
        assert_eq!(
            fire_due(&mut manager, &day(7)),
            [remind(T1, ReminderKind::Stale)]
        );
    }

    #[test]
    fn partner_progress_moves_all_deadlines() {
        let progress = DomainEvent::Ticket(TicketEvent::PartnerProgressReported(
            PartnerProgressReportedEvent {
                ticket_id: T1,
                partner_id: PartnerId(1),
                comment: "On site tomorrow".to_owned(),
                at: day(2),
            },
        ));
        let manager = manager_with([created(T1, day(0)), progress]);
        assert_eq!(
            manager.deadlines_of(&T1),
            [
                (STALE, day(5)),
                (CLOSING_SOON, day(11)),
                (TicketDeadline::AutoClose, day(12)),
            ]
        );
    }

    #[test]
    fn resource_check_and_handover_do_not_move_deadlines() {
        let checked = DomainEvent::Ticket(TicketEvent::ResourcesChecked(ResourcesCheckedEvent {
            ticket_id: T1,
            availability: ResourceAvailability::Unavailable,
            at: day(1),
        }));
        let handed = DomainEvent::Ticket(TicketEvent::HandedToPartner(HandedToPartnerEvent {
            ticket_id: T1,
            partner_id: PartnerId(1),
            at: day(1),
        }));
        let manager = manager_with([created(T1, day(0)), checked, handed]);
        assert_eq!(
            manager.deadlines_of(&T1),
            [
                (STALE, day(3)),
                (CLOSING_SOON, day(9)),
                (TicketDeadline::AutoClose, day(10)),
            ]
        );
    }

    #[test]
    fn reminder_event_does_not_move_deadlines() {
        let manager = manager_with([
            created(T1, day(0)),
            reminder_sent(T1, ReminderKind::Stale, day(3)),
        ]);
        assert_eq!(
            manager.deadlines_of(&T1),
            [
                (STALE, day(3)),
                (CLOSING_SOON, day(9)),
                (TicketDeadline::AutoClose, day(10)),
            ]
        );
    }

    #[test]
    fn auto_close_waits_for_the_event_before_finishing() {
        let mut manager = manager_with([created(T1, day(0))]);
        assert_eq!(
            fire_due(&mut manager, &day(10)).last(),
            Some(&auto_close(T1))
        );
        assert!(manager.is_running(&T1));
        assert_eq!(manager.on_event(&closed(T1, day(10))), []);
        assert!(!manager.is_running(&T1));
    }

    #[test]
    fn activity_after_rejected_auto_close_keeps_process_going() {
        let mut manager = manager_with([created(T1, day(0))]);
        fire_due(&mut manager, &day(10));
        // Агент назначен раньше, чем команда дошла до агрегата: тот её отклонит, а сага узнает
        // об активности из потока событий и назначит новые сроки.
        assert_eq!(manager.on_event(&assigned(T1, day(10))), []);
        assert_eq!(fire_due(&mut manager, &day(19)).len(), 2);
        assert_eq!(fire_due(&mut manager, &day(20)), [auto_close(T1)]);
    }

    #[test]
    fn reopening_starts_a_new_process() {
        let manager = manager_with([
            created(T1, day(0)),
            closed(T1, day(1)),
            reopened(T1, day(5)),
        ]);
        assert!(manager.is_running(&T1));
        assert_eq!(
            manager.deadlines_of(&T1),
            [
                (STALE, day(8)),
                (CLOSING_SOON, day(14)),
                (TicketDeadline::AutoClose, day(15)),
            ]
        );
    }

    #[test]
    fn tickets_have_independent_processes() {
        let mut manager =
            manager_with([created(T1, day(0)), created(T2, day(1)), closed(T1, day(1))]);
        assert!(manager.is_running(&T2));
        assert_eq!(
            fire_due(&mut manager, &day(4)),
            [remind(T2, ReminderKind::Stale)]
        );
    }

    #[test]
    fn other_events_start_nothing() {
        let tenant = TenantEvent::Registered(TenantRegisteredEvent {
            tenant_id: TenantId(1),
            tenant_name: "Acme".to_owned(),
        });
        let manager = manager_with([
            tenant.into(),
            assigned(T1, day(0)),
            closed(T1, day(0)),
            reminder_sent(T1, ReminderKind::Stale, day(0)),
        ]);
        assert!(!manager.is_running(&T1));
        assert_eq!(manager.next_deadline(), None);
    }

    /// Сага и настоящий сервис приложения: команды выполняются, события возвращаются в сагу.
    struct Process {
        clock: ManualClock,
        service: TicketService<ManualClock>,
        manager: Manager,
    }

    impl Process {
        async fn new() -> Self {
            let clock = ManualClock::default();
            let mut service = TicketService::new(clock.clone());
            service
                .register_tenant(RegisterTenantCommand::new("Acme"))
                .await
                .unwrap();
            service
                .register_user(RegisterUserCommand::new("Alice", Role::Agent))
                .await
                .unwrap();
            Self {
                clock,
                service,
                manager: Manager::default(),
            }
        }

        fn drain_events(&mut self) -> Vec<DomainEvent> {
            let events = self.service.take_outbox();
            for event in &events {
                self.manager.on_event(event);
            }
            events
        }

        /// Двигает время и выполняет всё, что наступило. Возвращает получившиеся события.
        async fn advance(&mut self, secs: u64) -> Vec<DomainEvent> {
            self.clock.advance(secs);
            self.drain_events();
            for command in fire_due(&mut self.manager, &self.clock.now()) {
                let result = match command {
                    LifecycleCommand::SendReminder(c) => self.service.handle_send_reminder(c).await,
                    LifecycleCommand::CloseTicket(c) => self.service.handle_close_ticket(c).await,
                };
                result.expect("aggregate accepts what the saga asks for at the right time");
            }
            self.drain_events()
        }

        async fn status(&self, ticket_id: TicketId) -> TicketStatus {
            self.service
                .ticket(ticket_id)
                .await
                .unwrap()
                .unwrap()
                .status()
        }
    }

    fn kinds(events: &[DomainEvent]) -> Vec<ReminderKind> {
        events
            .iter()
            .filter_map(|event| match event {
                DomainEvent::Ticket(TicketEvent::ReminderSent(e)) => Some(e.kind),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn aggregate_accepts_commands_exactly_on_saga_deadlines() {
        let mut process = Process::new().await;
        process
            .service
            .handle_create_ticket(CreateTicketCommand::new(
                "Billing",
                "Invoice",
                TenantId(1),
                CustomerTier::Bronze,
            ))
            .await
            .unwrap();

        assert_eq!(process.advance(DAY).await, []);
        process
            .service
            .handle_assign_agent(AssignAgentCommand {
                ticket_id: T1,
                agent_id: UserId(1),
            })
            .await
            .unwrap();
        process.drain_events();

        // Сроки считаются от назначения на первый день.
        assert_eq!(process.advance(3 * DAY - 1).await, []);
        let stale = process.advance(1).await;
        assert_eq!(kinds(&stale), [ReminderKind::Stale]);
        assert!(matches!(
            stale[0],
            DomainEvent::Ticket(TicketEvent::ReminderSent(ReminderSentEvent {
                agent_id: Some(UserId(1)),
                ..
            }))
        ));

        let closing_soon = process.advance(6 * DAY).await;
        assert_eq!(kinds(&closing_soon), [ReminderKind::ClosingSoon]);
        assert_eq!(process.status(T1).await, TicketStatus::InProgress);

        let closing = process.advance(DAY).await;
        assert!(matches!(
            closing[..],
            [DomainEvent::Ticket(TicketEvent::Closed(
                TicketClosedEvent {
                    reason: CloseReason::Inactivity,
                    agent_id: Some(UserId(1)),
                    ..
                }
            ))]
        ));
        assert_eq!(process.status(T1).await, TicketStatus::Closed);
        assert!(!process.manager.is_running(&T1));
    }

    #[tokio::test]
    async fn unassigned_ticket_is_reminded_to_admins_and_closed() {
        let mut process = Process::new().await;
        process
            .service
            .handle_create_ticket(CreateTicketCommand::new(
                "Billing",
                "Invoice",
                TenantId(1),
                CustomerTier::Bronze,
            ))
            .await
            .unwrap();

        let events = process.advance(3 * DAY).await;
        assert!(matches!(
            events[..],
            [DomainEvent::Ticket(TicketEvent::ReminderSent(
                ReminderSentEvent { agent_id: None, .. }
            ))]
        ));
        process.advance(7 * DAY).await;
        assert_eq!(process.status(T1).await, TicketStatus::Closed);
    }

    #[tokio::test]
    async fn service_closed_ticket_stops_process_and_reopened_gets_new_one() {
        let mut process = Process::new().await;
        process
            .service
            .handle_create_ticket(CreateTicketCommand::new(
                "Billing",
                "Invoice",
                TenantId(1),
                CustomerTier::Bronze,
            ))
            .await
            .unwrap();
        process
            .service
            .handle_close_ticket(CloseTicketCommand {
                ticket_id: T1,
                reason: CloseReason::Resolved,
            })
            .await
            .unwrap();
        assert_eq!(process.advance(30 * DAY).await, []);
        assert!(!process.manager.is_running(&T1));

        process
            .service
            .handle_reopen_ticket(ReopenTicketCommand { ticket_id: T1 })
            .await
            .unwrap();
        process.drain_events();
        assert_eq!(
            kinds(&process.advance(3 * DAY).await),
            [ReminderKind::Stale]
        );
    }

    #[tokio::test]
    async fn stale_auto_close_is_rejected_by_aggregate_after_activity() {
        let mut process = Process::new().await;
        process
            .service
            .handle_create_ticket(CreateTicketCommand::new(
                "Billing",
                "Invoice",
                TenantId(1),
                CustomerTier::Bronze,
            ))
            .await
            .unwrap();
        process.drain_events();
        process.clock.advance(10 * DAY);

        // Дедлайн уже сработал, но агент назначен раньше, чем команда дошла до агрегата.
        let commands = fire_due(&mut process.manager, &day(10));
        assert_eq!(commands.last(), Some(&auto_close(T1)));
        process
            .service
            .handle_assign_agent(AssignAgentCommand {
                ticket_id: T1,
                agent_id: UserId(1),
            })
            .await
            .unwrap();
        let LifecycleCommand::CloseTicket(close) = commands[2] else {
            panic!("the last command closes the ticket");
        };
        assert_eq!(
            process.service.handle_close_ticket(close).await,
            Err(ServiceError::Ticket(TicketError::StillActive {
                idle_secs: 0
            }))
        );
        assert_eq!(process.status(T1).await, TicketStatus::InProgress);
    }
}
