use std::time::Duration;

use framework::cqrs::{
    command_bus::CommandBus,
    envelope::Version,
    event_source::{EventSource, Subscription},
};
use framework::saga::manager::SagaManager;
use ticketing::application::{
    clock::Clock,
    ticket_lifecycle::{LifecycleCommand, TicketLifecycleSaga},
};
use tokio::time::{self, MissedTickBehavior};

use crate::handle::{CommandActorHandle, CommandError};

/// Запускает оркестратор жизненного цикла тикетов: слушает события с начала журнала,
/// раз в `tick` проверяет дедлайны по часам `clock` и отправляет команды саги актору.
///
/// Перед каждым сработавшим дедлайном разбираются все накопившиеся события: если тикет
/// только что закрыли или назначили на него агента, устаревший дедлайн будет снят раньше,
/// чем уйдёт его команда. Актор публикует события до ответа на команду, поэтому результат
/// собственной команды сага тоже увидит ещё до следующего дедлайна. Команды отправляются
/// по одной с ожиданием ответа: даже лавина дедлайнов занимает одно место в очереди актора.
///
/// Отказы уходят в `on_error`: для саги это нормальный исход, если дедлайн устарел.
/// Завершается `Ok`, когда актор закрывает поток событий.
pub async fn run_ticket_lifecycle<C: Clock + Send>(
    bus: CommandActorHandle,
    clock: C,
    tick: Duration,
    mut on_error: impl FnMut(CommandError) + Send,
) -> Result<(), CommandError> {
    let mut saga = SagaManager::<TicketLifecycleSaga>::default();
    let mut events = bus.subscribe(Version(0)).await?;
    let mut ticks = time::interval_at(time::Instant::now() + tick, tick);
    // Если задача отстала, пропущенные проверки не нагоняем пачкой: хватит одной.
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            event = events.next() => {
                let Some(envelope) = event else {
                    return Ok(());
                };
                let commands = saga.on_event(&envelope.event);
                send_all(&bus, commands, &mut on_error).await;
                continue;
            }
            _ = ticks.tick() => {}
        }
        loop {
            while let Some(envelope) = events.try_next() {
                let commands = saga.on_event(&envelope.event);
                send_all(&bus, commands, &mut on_error).await;
            }
            let Some(commands) = saga.fire_next(&clock.now()) else {
                break;
            };
            send_all(&bus, commands, &mut on_error).await;
        }
    }
}

async fn send_all(
    bus: &CommandActorHandle,
    commands: Vec<LifecycleCommand>,
    on_error: &mut (impl FnMut(CommandError) + Send),
) {
    for command in commands {
        if let Err(e) = bus.send(command).await {
            on_error(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::EventSubscription;
    use ticketing::application::{
        clock::ManualClock,
        commands::{
            AssignAgentCommand, CloseTicketCommand, CreateTicketCommand, RegisterTenantCommand,
            RegisterUserCommand, ReopenTicketCommand,
        },
        ticket_service::TicketService,
    };
    use ticketing::domain::sla::CustomerTier;
    use ticketing::domain::{
        DomainEvent,
        tenant::TenantId,
        ticket::{CloseReason, ReminderKind, TicketEvent, TicketId},
        time::Timestamp,
        user::{Role, UserId},
    };

    const DAY: u64 = 24 * 60 * 60;
    const TICK: Duration = Duration::from_secs(1);
    const T1: TicketId = TicketId(1);
    const T2: TicketId = TicketId(2);
    const ALICE: UserId = UserId(1);

    /// Что случилось с тикетом — коротко, чтобы сценарии читались как список.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Seen {
        Reminder(TicketId, ReminderKind, Option<UserId>),
        AutoClosed(TicketId),
    }

    struct Scenario {
        bus: CommandActorHandle,
        clock: ManualClock,
        /// События после подготовки сценария.
        events: EventSubscription,
        errors: Arc<Mutex<Vec<CommandError>>>,
    }

    /// Актор с тенантом и агентом и запущенный оркестратор. Время tokio остановлено:
    /// оно само идёт вперёд, когда всем задачам нечего делать.
    async fn scenario() -> Scenario {
        let clock = ManualClock::default();
        let bus = crate::spawn(TicketService::new(clock.clone()));
        bus.send(RegisterTenantCommand::new("Acme")).await.unwrap();
        bus.send(RegisterUserCommand::new("Alice", Role::Agent))
            .await
            .unwrap();
        let events = bus.subscribe(Version(2)).await.unwrap();
        let errors = Arc::new(Mutex::new(Vec::new()));
        let collected = errors.clone();
        tokio::spawn(run_ticket_lifecycle(
            bus.clone(),
            clock.clone(),
            TICK,
            move |e| collected.lock().unwrap().push(e),
        ));
        Scenario {
            bus,
            clock,
            events,
            errors,
        }
    }

    impl Scenario {
        async fn create_ticket(&self) {
            self.bus
                .send(CreateTicketCommand::new(
                    "Billing",
                    "Invoice",
                    TenantId(1),
                    CustomerTier::Bronze,
                ))
                .await
                .unwrap();
        }

        async fn assign(&self, ticket_id: TicketId) {
            self.bus
                .send(AssignAgentCommand {
                    ticket_id,
                    agent_id: ALICE,
                })
                .await
                .unwrap();
        }

        async fn close(&self, ticket_id: TicketId) {
            self.bus
                .send(CloseTicketCommand {
                    ticket_id,
                    reason: CloseReason::Resolved,
                })
                .await
                .unwrap();
        }

        /// Двигает часы и даёт оркестратору сработать; возвращает реакции саги.
        async fn advance(&mut self, secs: u64) -> Vec<Seen> {
            self.clock.advance(secs);
            time::sleep(3 * TICK).await;
            let mut seen = Vec::new();
            while let Some(envelope) = self.events.try_next() {
                match envelope.event {
                    DomainEvent::Ticket(TicketEvent::ReminderSent(e)) => {
                        seen.push(Seen::Reminder(e.ticket_id, e.kind, e.agent_id));
                    }
                    DomainEvent::Ticket(TicketEvent::Closed(e))
                        if e.reason == CloseReason::Inactivity =>
                    {
                        seen.push(Seen::AutoClosed(e.ticket_id));
                    }
                    _ => {}
                }
            }
            seen
        }

        fn errors(&self) -> Vec<CommandError> {
            self.errors.lock().unwrap().clone()
        }
    }

    #[tokio::test(start_paused = true)]
    async fn untouched_ticket_is_reminded_and_closed_on_schedule() {
        let mut s = scenario().await;
        s.create_ticket().await;

        assert_eq!(s.advance(3 * DAY - 1).await, []);
        assert_eq!(
            s.advance(1).await,
            [Seen::Reminder(T1, ReminderKind::Stale, None)]
        );
        assert_eq!(s.advance(6 * DAY - 1).await, []);
        assert_eq!(
            s.advance(1).await,
            [Seen::Reminder(T1, ReminderKind::ClosingSoon, None)]
        );
        assert_eq!(s.advance(DAY - 1).await, []);
        assert_eq!(s.advance(1).await, [Seen::AutoClosed(T1)]);

        // Процесс завершён: дальше ничего не происходит.
        assert_eq!(s.advance(30 * DAY).await, []);
        assert_eq!(s.errors(), []);
    }

    #[tokio::test(start_paused = true)]
    async fn ticket_closed_before_deadline_gets_nothing() {
        let mut s = scenario().await;
        s.create_ticket().await;
        assert_eq!(s.advance(DAY).await, []);
        s.close(T1).await;

        assert_eq!(s.advance(30 * DAY).await, []);
        assert_eq!(s.errors(), []);
    }

    #[tokio::test(start_paused = true)]
    async fn assignment_moves_deadlines_and_reminder_goes_to_agent() {
        let mut s = scenario().await;
        s.create_ticket().await;
        assert_eq!(s.advance(2 * DAY).await, []);
        s.assign(T1).await;

        // Без назначения напоминание ушло бы на третий день, теперь — на пятый.
        assert_eq!(s.advance(3 * DAY - 1).await, []);
        assert_eq!(
            s.advance(1).await,
            [Seen::Reminder(T1, ReminderKind::Stale, Some(ALICE))]
        );
        assert_eq!(
            s.advance(7 * DAY).await,
            [
                Seen::Reminder(T1, ReminderKind::ClosingSoon, Some(ALICE)),
                Seen::AutoClosed(T1),
            ]
        );
        assert_eq!(s.errors(), []);
    }

    #[tokio::test(start_paused = true)]
    async fn reopened_ticket_starts_process_again() {
        let mut s = scenario().await;
        s.create_ticket().await;
        s.close(T1).await;
        assert_eq!(s.advance(20 * DAY).await, []);
        s.bus
            .send(ReopenTicketCommand { ticket_id: T1 })
            .await
            .unwrap();

        assert_eq!(
            s.advance(3 * DAY).await,
            [Seen::Reminder(T1, ReminderKind::Stale, None)]
        );
        assert_eq!(s.errors(), []);
    }

    #[tokio::test(start_paused = true)]
    async fn overdue_deadlines_run_in_order_without_rejections() {
        let mut s = scenario().await;
        s.create_ticket().await;
        s.create_ticket().await;

        assert_eq!(
            s.advance(50 * DAY).await,
            [
                Seen::Reminder(T1, ReminderKind::Stale, None),
                Seen::Reminder(T2, ReminderKind::Stale, None),
                Seen::Reminder(T1, ReminderKind::ClosingSoon, None),
                Seen::Reminder(T2, ReminderKind::ClosingSoon, None),
                Seen::AutoClosed(T1),
                Seen::AutoClosed(T2),
            ]
        );
        assert_eq!(s.errors(), []);
    }

    /// Срок уже наступил, а проверка ещё не успела: закрытие снимает дедлайн до отправки команды,
    /// поэтому агрегату не приходится её отклонять.
    #[tokio::test(start_paused = true)]
    async fn event_arriving_before_the_tick_cancels_due_deadline() {
        let mut s = scenario().await;
        s.create_ticket().await;
        s.create_ticket().await;
        assert_eq!(s.advance(0).await, []);

        // Уходим с границы тика, чтобы закрытие гарантированно успело раньше проверки.
        time::sleep(TICK / 2).await;
        s.clock.advance(3 * DAY);
        s.close(T1).await;

        assert_eq!(
            s.advance(0).await,
            [Seen::Reminder(T2, ReminderKind::Stale, None)]
        );
        assert_eq!(s.errors(), []);
    }

    /// Часы, которые при первом обращении запускают закрытие тикета: оно окажется в очереди актора
    /// сразу за командой первого сработавшего дедлайна.
    struct ClosingOnFirstLook {
        clock: ManualClock,
        bus: CommandActorHandle,
        target: Mutex<Option<TicketId>>,
    }

    impl Clock for ClosingOnFirstLook {
        fn now(&self) -> Timestamp {
            if let Some(ticket_id) = self.target.lock().unwrap().take() {
                let bus = self.bus.clone();
                tokio::spawn(async move {
                    let close = CloseTicketCommand {
                        ticket_id,
                        reason: CloseReason::Resolved,
                    };
                    bus.send(close).await.unwrap();
                });
            }
            self.clock.now()
        }
    }

    #[tokio::test(start_paused = true)]
    async fn event_between_due_deadlines_cancels_the_next_one() {
        let clock = ManualClock::default();
        let bus = crate::spawn(TicketService::new(clock.clone()));
        bus.send(RegisterTenantCommand::new("Acme")).await.unwrap();
        for _ in 0..2 {
            bus.send(CreateTicketCommand::new(
                "Billing",
                "Invoice",
                TenantId(1),
                CustomerTier::Bronze,
            ))
            .await
            .unwrap();
        }
        let mut events = bus.subscribe(Version(3)).await.unwrap();
        clock.advance(3 * DAY);

        let errors = Arc::new(Mutex::new(Vec::new()));
        let collected = errors.clone();
        tokio::spawn(run_ticket_lifecycle(
            bus.clone(),
            ClosingOnFirstLook {
                clock,
                bus,
                target: Mutex::new(Some(T2)),
            },
            TICK,
            move |e| collected.lock().unwrap().push(e),
        ));
        time::sleep(3 * TICK).await;

        let mut reminded = Vec::new();
        while let Some(envelope) = events.try_next() {
            if let DomainEvent::Ticket(TicketEvent::ReminderSent(e)) = envelope.event {
                reminded.push(e.ticket_id);
            }
        }
        assert_eq!(reminded, [T1]);
        // Дедлайн второго тикета снят ещё до отправки, поэтому агрегату нечего отклонять.
        assert_eq!(*errors.lock().unwrap(), []);
    }

    #[tokio::test(start_paused = true)]
    async fn tickets_created_before_start_are_picked_up_from_history() {
        let clock = ManualClock::default();
        let bus = crate::spawn(TicketService::new(clock.clone()));
        bus.send(RegisterTenantCommand::new("Acme")).await.unwrap();
        bus.send(CreateTicketCommand::new(
            "Billing",
            "Invoice",
            TenantId(1),
            CustomerTier::Bronze,
        ))
        .await
        .unwrap();
        let mut events = bus.subscribe(Version(2)).await.unwrap();
        clock.advance(10 * DAY);

        tokio::spawn(run_ticket_lifecycle(bus.clone(), clock, TICK, |_| {}));
        time::timeout(Duration::from_secs(60), async {
            loop {
                let envelope = events.next().await.unwrap();
                if matches!(envelope.event, DomainEvent::Ticket(TicketEvent::Closed(_))) {
                    break;
                }
            }
        })
        .await
        .expect("the ticket from history is closed by its deadline");
    }
}
