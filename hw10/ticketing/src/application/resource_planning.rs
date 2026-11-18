//! Шаги 2 и 3 бизнес-процесса — проверить ресурсы в ERP и, если своих нет, передать тикет
//! партнёру.
//!
//! На каждый новый или открытый заново тикет цикл спрашивает ERP, записывает ответ в тикет
//! и, если ресурсов нет, передаёт тикет партнёру, который берёт эту категорию. Оба решения
//! принимает агрегат: за время запроса тикет могли закрыть, и тогда команды он отклонит.

use std::error::Error;
use std::fmt;

use framework::cqrs::{
    command_bus::CommandBus,
    envelope::Version,
    event_source::{EventSource, Subscription},
};

use super::{
    commands::{HandToPartnerCommand, RecordResourceCheckCommand},
    partners::PartnerRoster,
    ports::{ExternalError, Inventory},
};
use crate::domain::{
    DomainEvent,
    ticket::{ResourceAvailability, TicketEvent, TicketId},
};

/// Что не получилось при планировании ресурсов тикета.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningError<E> {
    /// ERP не ответила: тикет остаётся без проверки ресурсов.
    Inventory {
        ticket_id: TicketId,
        error: ExternalError,
    },
    /// Ресурсов нет, но ни один партнёр эту категорию не берёт: решать придётся своими силами.
    NoPartner {
        ticket_id: TicketId,
        category: String,
    },
    /// Командная сторона отклонила результат проверки или передачу.
    Command(E),
}

impl<E: fmt::Display> fmt::Display for PlanningError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inventory { ticket_id, error } => {
                write!(f, "resources for {ticket_id} are not checked: {error}")
            }
            Self::NoPartner {
                ticket_id,
                category,
            } => write!(
                f,
                "no partner takes '{category}' tickets, {ticket_id} stays in-house"
            ),
            Self::Command(e) => e.fmt(f),
        }
    }
}

impl<E: Error> Error for PlanningError<E> {}

/// Тикет, ресурсы для которого надо проверить, и его категория.
fn needs_planning(event: &DomainEvent) -> Option<(TicketId, &str)> {
    match event {
        DomainEvent::Ticket(TicketEvent::Created(e)) => Some((e.ticket_id, &e.category)),
        DomainEvent::Ticket(TicketEvent::Reopened(e)) => Some((e.ticket_id, &e.category)),
        _ => None,
    }
}

/// Планирует ресурсы для тикетов из событий с версией больше `after`, по одному и по порядку.
///
/// Ни сбой ERP, ни отказ агрегата цикл не останавливают: они уходят в `on_error`.
/// Завершается `Ok`, когда источник закрывает поток.
pub async fn run_resource_planning<S, I, B, E>(
    source: &S,
    after: Version,
    inventory: &I,
    partners: &PartnerRoster,
    bus: &B,
    mut on_error: impl FnMut(PlanningError<E>),
) -> Result<(), S::Error>
where
    S: EventSource<DomainEvent>,
    I: Inventory,
    B: CommandBus<RecordResourceCheckCommand, Error = E>
        + CommandBus<HandToPartnerCommand, Error = E>,
{
    let mut events = source.subscribe(after).await?;
    while let Some(envelope) = events.next().await {
        let Some((ticket_id, category)) = needs_planning(&envelope.event) else {
            continue;
        };
        if let Err(e) = plan(ticket_id, category, inventory, partners, bus).await {
            on_error(e);
        }
    }
    Ok(())
}

async fn plan<I, B, E>(
    ticket_id: TicketId,
    category: &str,
    inventory: &I,
    partners: &PartnerRoster,
    bus: &B,
) -> Result<(), PlanningError<E>>
where
    I: Inventory,
    B: CommandBus<RecordResourceCheckCommand, Error = E>
        + CommandBus<HandToPartnerCommand, Error = E>,
{
    let availability = inventory
        .check_resources(category)
        .await
        .map_err(|error| PlanningError::Inventory { ticket_id, error })?;
    let record = RecordResourceCheckCommand {
        ticket_id,
        availability,
    };
    bus.send(record).await.map_err(PlanningError::Command)?;
    if availability == ResourceAvailability::Available {
        return Ok(());
    }
    let partner = partners
        .for_category(category)
        .ok_or_else(|| PlanningError::NoPartner {
            ticket_id,
            category: category.to_owned(),
        })?;
    let hand = HandToPartnerCommand {
        ticket_id,
        partner_id: partner.partner_id,
    };
    bus.send(hand).await.map_err(PlanningError::Command)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use framework::cqrs::envelope::Committed;

    use super::*;
    use crate::application::{partners::Partner, testing::Journal};
    use crate::domain::{
        partner::PartnerId,
        sla::{CustomerTier, Sla},
        tenant::TenantId,
        ticket::{TicketCreatedEvent, TicketReopenedEvent},
        time::Timestamp,
    };

    const FIXIT: PartnerId = PartnerId(1);

    fn created(ticket: usize, category: &str) -> DomainEvent {
        TicketEvent::Created(TicketCreatedEvent {
            ticket_id: TicketId(ticket),
            category: category.to_owned(),
            description: "Something broke".to_owned(),
            tenant_id: TenantId(1),
            tier: CustomerTier::Bronze,
            sla: Sla::for_tier(CustomerTier::Bronze, Timestamp(0)),
            at: Timestamp(0),
        })
        .into()
    }

    fn reopened(ticket: usize, category: &str) -> DomainEvent {
        TicketEvent::Reopened(TicketReopenedEvent {
            ticket_id: TicketId(ticket),
            tenant_id: TenantId(1),
            category: category.to_owned(),
            sla: Sla::for_tier(CustomerTier::Bronze, Timestamp(0)),
            at: Timestamp(0),
        })
        .into()
    }

    /// ERP по категориям: Hardware — нет на складе, Network — ERP не отвечает, остальное есть.
    struct Warehouse;

    impl Inventory for Warehouse {
        async fn check_resources(
            &self,
            category: &str,
        ) -> Result<ResourceAvailability, ExternalError> {
            match category {
                "Hardware" | "Cables" => Ok(ResourceAvailability::Unavailable),
                "Network" => Err(ExternalError::Unavailable {
                    system: "ERP",
                    reason: "timeout".to_owned(),
                }),
                _ => Ok(ResourceAvailability::Available),
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Sent {
        Record(RecordResourceCheckCommand),
        Hand(HandToPartnerCommand),
    }

    /// Шина, которая запоминает команды; передачу тикета 13 отклоняет.
    #[derive(Default)]
    struct Bus(Mutex<Vec<Sent>>);

    impl CommandBus<RecordResourceCheckCommand> for Bus {
        type Event = DomainEvent;
        type Error = &'static str;

        async fn send(
            &self,
            command: RecordResourceCheckCommand,
        ) -> Result<Committed<DomainEvent>, &'static str> {
            self.0.lock().unwrap().push(Sent::Record(command));
            Ok(Committed {
                version: Version(0),
                events: Vec::new(),
            })
        }
    }

    impl CommandBus<HandToPartnerCommand> for Bus {
        type Event = DomainEvent;
        type Error = &'static str;

        async fn send(
            &self,
            command: HandToPartnerCommand,
        ) -> Result<Committed<DomainEvent>, &'static str> {
            if command.ticket_id == TicketId(13) {
                return Err("ticket is closed");
            }
            self.0.lock().unwrap().push(Sent::Hand(command));
            Ok(Committed {
                version: Version(0),
                events: Vec::new(),
            })
        }
    }

    fn roster() -> PartnerRoster {
        PartnerRoster::new(vec![Partner::new(FIXIT, "FixIt", &["Hardware"])])
    }

    async fn run(events: Vec<DomainEvent>) -> (Vec<Sent>, Vec<PlanningError<&'static str>>) {
        let bus = Bus::default();
        let mut errors = Vec::new();
        run_resource_planning(
            &Journal(events),
            Version(0),
            &Warehouse,
            &roster(),
            &bus,
            |e| errors.push(e),
        )
        .await
        .unwrap();
        (bus.0.into_inner().unwrap(), errors)
    }

    fn record(ticket: usize, availability: ResourceAvailability) -> Sent {
        Sent::Record(RecordResourceCheckCommand {
            ticket_id: TicketId(ticket),
            availability,
        })
    }

    #[tokio::test]
    async fn available_resources_are_recorded_and_ticket_stays_in_house() {
        let (sent, errors) = run(vec![created(1, "Billing")]).await;
        assert_eq!(sent, [record(1, ResourceAvailability::Available)]);
        assert_eq!(errors, []);
    }

    #[tokio::test]
    async fn missing_resources_send_ticket_to_partner_of_its_category() {
        let (sent, errors) = run(vec![created(1, "Hardware")]).await;
        assert_eq!(
            sent,
            [
                record(1, ResourceAvailability::Unavailable),
                Sent::Hand(HandToPartnerCommand {
                    ticket_id: TicketId(1),
                    partner_id: FIXIT,
                }),
            ]
        );
        assert_eq!(errors, []);
    }

    #[tokio::test]
    async fn reopened_ticket_is_planned_again() {
        let (sent, _) = run(vec![reopened(2, "Billing")]).await;
        assert_eq!(sent, [record(2, ResourceAvailability::Available)]);
    }

    #[tokio::test]
    async fn erp_failure_leaves_ticket_unchecked_and_loop_goes_on() {
        let (sent, errors) = run(vec![created(1, "Network"), created(2, "Billing")]).await;
        assert_eq!(sent, [record(2, ResourceAvailability::Available)]);
        assert!(matches!(
            errors.as_slice(),
            [PlanningError::Inventory {
                ticket_id: TicketId(1),
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn without_partner_for_category_ticket_stays_in_house() {
        let (sent, errors) = run(vec![created(1, "Cables")]).await;
        assert_eq!(sent, [record(1, ResourceAvailability::Unavailable)]);
        assert_eq!(
            errors,
            [PlanningError::NoPartner {
                ticket_id: TicketId(1),
                category: "Cables".to_owned(),
            }]
        );
    }

    #[tokio::test]
    async fn rejected_handover_is_reported() {
        let (_, errors) = run(vec![created(13, "Hardware")]).await;
        assert_eq!(errors, [PlanningError::Command("ticket is closed")]);
    }
}
