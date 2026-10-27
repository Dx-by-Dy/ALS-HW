use std::time::Duration;

use framework::cqrs::command_bus::CommandBus;
use tokio::time::{self, MissedTickBehavior};

use crate::command::Command;
use crate::handle::{CommandActorHandle, CommandError};

/// Отправляет команду каждые `period`, пока актор работает. Первая отправка — через `period`.
///
/// Так время само становится источником команд: например, `CloseInactiveTickets` раз в минуту
/// закрывает тикеты, у которых истёк таймаут. Отклонённая команда расписание не прерывает.
pub async fn repeat(bus: CommandActorHandle, period: Duration, command: Command) {
    let mut ticks = time::interval_at(time::Instant::now() + period, period);
    // Если актор был занят дольше периода, пропущенные запуски не нагоняем пачкой.
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        if let Err(CommandError::Unavailable) = bus.send(command.clone()).await {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use framework::cqrs::{
        envelope::Version,
        event_source::{EventSource, Subscription},
    };
    use ticketing::application::{
        clock::ManualClock,
        commands::{CreateTicketCommand, RegisterTenantCommand},
        ticket_service::TicketService,
    };
    use ticketing::domain::{
        DomainEvent,
        tenant::TenantId,
        ticket::{CloseReason, INACTIVITY_TIMEOUT_SECS, TicketEvent},
    };

    const MINUTE: Duration = Duration::from_secs(60);

    #[tokio::test(start_paused = true)]
    async fn inactive_tickets_are_closed_on_schedule() {
        let clock = ManualClock::default();
        let bus = crate::spawn(TicketService::new(clock.clone()));
        bus.send(RegisterTenantCommand::new("Acme")).await.unwrap();
        bus.send(CreateTicketCommand::new("Billing", "Invoice", TenantId(1)))
            .await
            .unwrap();
        let mut events = bus.subscribe(Version(2)).await.unwrap();
        tokio::spawn(repeat(bus.clone(), MINUTE, Command::CloseInactiveTickets));

        // Пока таймаут не истёк, расписание срабатывает впустую.
        time::sleep(10 * MINUTE).await;
        assert_eq!(events.try_next(), None);

        clock.advance(INACTIVITY_TIMEOUT_SECS);
        let started = time::Instant::now();
        let closed = events.next().await.unwrap();
        assert!(matches!(
            closed.event,
            DomainEvent::Ticket(TicketEvent::Closed(ref e)) if e.reason == CloseReason::Inactivity
        ));
        assert!(started.elapsed() <= MINUTE);
    }
}
