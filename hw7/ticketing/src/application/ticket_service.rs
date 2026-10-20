use std::num::NonZeroUsize;

use super::{
    clock::Clock,
    commands::{
        ApplyDiscountCommand, AssignAgentCommand, CloseTicketCommand, CreateTicketCommand,
        RegisterTenantCommand, RegisterUserCommand,
    },
    error::ServiceError,
    open_tickets::OpenTickets,
    repository::Repository,
};
use crate::core::aggregate::Aggregate;
use crate::domain::{
    tenant::{Tenant, TenantCommand, TenantEvent, TenantId},
    ticket::{CloseReason, Ticket, TicketCommand, TicketEvent, TicketId},
    user::{User, UserCommand, UserEvent, UserId},
};
use crate::infrastructure::{
    in_memory_event_store::InMemoryEventStore, in_memory_snapshot_store::InMemorySnapshotStore,
};

type InMemoryRepository<A> =
    Repository<A, InMemoryEventStore<<A as Aggregate>::Event>, InMemorySnapshotStore<A>>;

const SNAPSHOT_EVERY: NonZeroUsize = NonZeroUsize::new(2).unwrap();

fn in_memory_repository<A>() -> InMemoryRepository<A>
where
    A: crate::core::event_sourced::EventSourced + Default + Clone,
    A::Event: Clone,
{
    Repository::new(
        InMemoryEventStore::default(),
        InMemorySnapshotStore::default(),
        SNAPSHOT_EVERY,
    )
}

/// Сервис приложения: принимает внешние команды, достраивает их до команд агрегатов
/// (идентификаторы, время, данные других агрегатов) и выполняет через репозитории.
/// Каждый метод возвращает событие, которое было записано в стрим.
pub struct TicketService<C> {
    clock: C,
    tickets: InMemoryRepository<Option<Ticket>>,
    tenants: InMemoryRepository<Option<Tenant>>,
    users: InMemoryRepository<Option<User>>,
    open_tickets: OpenTickets,
    next_ticket_id: usize,
    next_tenant_id: usize,
    next_user_id: usize,
}

impl<C: Clock> TicketService<C> {
    pub fn new(clock: C) -> Self {
        Self {
            clock,
            tickets: in_memory_repository(),
            tenants: in_memory_repository(),
            users: in_memory_repository(),
            open_tickets: OpenTickets::default(),
            next_ticket_id: 1,
            next_tenant_id: 1,
            next_user_id: 1,
        }
    }

    pub fn register_tenant(
        &mut self,
        command: RegisterTenantCommand,
    ) -> Result<TenantEvent, ServiceError> {
        let tenant_id = TenantId(self.next_tenant_id);
        let command = TenantCommand::Register {
            tenant_id,
            tenant_name: command.tenant_name,
        };
        let event = self.tenants.execute(tenant_id.into(), &command)?;
        self.next_tenant_id += 1;
        Ok(event)
    }

    pub fn register_user(
        &mut self,
        command: RegisterUserCommand,
    ) -> Result<UserEvent, ServiceError> {
        let user_id = UserId(self.next_user_id);
        let command = UserCommand::Register {
            user_id,
            user_name: command.user_name,
            role: command.role,
        };
        let event = self.users.execute(user_id.into(), &command)?;
        self.next_user_id += 1;
        Ok(event)
    }

    pub fn handle_create_ticket(
        &mut self,
        command: CreateTicketCommand,
    ) -> Result<TicketEvent, ServiceError> {
        if self.tenant(command.tenant_id)?.is_none() {
            return Err(ServiceError::UnknownTenant(command.tenant_id));
        }
        let ticket_id = TicketId(self.next_ticket_id);
        let event = self.execute_ticket(
            ticket_id,
            TicketCommand::Create {
                ticket_id,
                category: command.category,
                description: command.description,
                tenant_id: command.tenant_id,
                at: self.clock.now(),
            },
        )?;
        self.next_ticket_id += 1;
        Ok(event)
    }

    pub fn handle_assign_agent(
        &mut self,
        command: AssignAgentCommand,
    ) -> Result<TicketEvent, ServiceError> {
        let agent = self
            .user(command.agent_id)?
            .ok_or(ServiceError::UnknownUser(command.agent_id))?;
        let at = self.clock.now();
        self.execute_ticket(command.ticket_id, TicketCommand::AssignAgent { agent, at })
    }

    pub fn handle_close_ticket(
        &mut self,
        command: CloseTicketCommand,
    ) -> Result<TicketEvent, ServiceError> {
        let at = self.clock.now();
        self.execute_ticket(
            command.ticket_id,
            TicketCommand::Close {
                reason: command.reason,
                at,
            },
        )
    }

    /// Закрывает все тикеты, простоявшие без активности дольше таймаута.
    pub fn close_inactive_tickets(&mut self) -> Result<Vec<TicketEvent>, ServiceError> {
        let now = self.clock.now();
        self.open_tickets
            .inactive_at(now)
            .into_iter()
            .map(|ticket_id| {
                let command = TicketCommand::Close {
                    reason: CloseReason::Inactivity,
                    at: now,
                };
                self.execute_ticket(ticket_id, command)
            })
            .collect()
    }

    /// Пересчитывает скидку тенанта по текущему числу его открытых тикетов.
    pub fn handle_apply_discount(
        &mut self,
        command: ApplyDiscountCommand,
    ) -> Result<TenantEvent, ServiceError> {
        let open_tickets = self.open_tickets.count_for_tenant(command.tenant_id);
        let command_for_tenant = TenantCommand::ApplyDiscount { open_tickets };
        Ok(self
            .tenants
            .execute(command.tenant_id.into(), &command_for_tenant)?)
    }

    pub fn ticket(&self, ticket_id: TicketId) -> Result<Option<Ticket>, ServiceError> {
        Ok(self.tickets.load(ticket_id.into())?)
    }

    pub fn tenant(&self, tenant_id: TenantId) -> Result<Option<Tenant>, ServiceError> {
        Ok(self.tenants.load(tenant_id.into())?)
    }

    pub fn user(&self, user_id: UserId) -> Result<Option<User>, ServiceError> {
        Ok(self.users.load(user_id.into())?)
    }

    fn execute_ticket(
        &mut self,
        ticket_id: TicketId,
        command: TicketCommand,
    ) -> Result<TicketEvent, ServiceError> {
        let event = self.tickets.execute(ticket_id.into(), &command)?;
        self.open_tickets.project(&event);
        Ok(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::clock::ManualClock;
    use crate::domain::{
        tenant::{DiscountAppliedEvent, DiscountRate, TenantError},
        ticket::{
            AgentAssignedEvent, INACTIVITY_TIMEOUT_SECS, TicketClosedEvent, TicketCreatedEvent,
            TicketError, TicketStatus,
        },
        time::Timestamp,
        user::Role,
    };

    const DAY: u64 = 24 * 60 * 60;

    struct Fixture {
        clock: ManualClock,
        service: TicketService<ManualClock>,
        tenant_id: TenantId,
        agent_id: UserId,
        customer_id: UserId,
    }

    fn fixture() -> Fixture {
        let clock = ManualClock::default();
        let mut service = TicketService::new(clock.clone());
        let tenant_id = service
            .register_tenant(RegisterTenantCommand::new("Acme"))
            .unwrap()
            .tenant_id();
        let agent_id = service
            .register_user(RegisterUserCommand::new("Alice", Role::Agent))
            .unwrap()
            .user_id();
        let customer_id = service
            .register_user(RegisterUserCommand::new("Bob", Role::Customer))
            .unwrap()
            .user_id();
        Fixture {
            clock,
            service,
            tenant_id,
            agent_id,
            customer_id,
        }
    }

    impl Fixture {
        fn create_ticket(&mut self) -> TicketId {
            let command =
                CreateTicketCommand::new("Technical Issue", "Problem description", self.tenant_id);
            self.service
                .handle_create_ticket(command)
                .unwrap()
                .ticket_id()
        }

        fn assign(
            &mut self,
            ticket_id: TicketId,
            agent_id: UserId,
        ) -> Result<TicketEvent, ServiceError> {
            self.service.handle_assign_agent(AssignAgentCommand {
                ticket_id,
                agent_id,
            })
        }

        fn status(&self, ticket_id: TicketId) -> TicketStatus {
            self.service
                .ticket(ticket_id)
                .unwrap()
                .expect("ticket exists")
                .status()
        }

        fn apply_discount(&mut self) -> Result<TenantEvent, ServiceError> {
            self.service.handle_apply_discount(ApplyDiscountCommand {
                tenant_id: self.tenant_id,
            })
        }
    }

    #[test]
    fn create_ticket_emits_created_event() {
        let mut f = fixture();
        f.clock.advance(10);
        let command =
            CreateTicketCommand::new("Technical Issue", "Problem description", f.tenant_id);
        assert_eq!(
            f.service.handle_create_ticket(command),
            Ok(TicketEvent::Created(TicketCreatedEvent {
                ticket_id: TicketId(1),
                category: "Technical Issue".to_owned(),
                description: "Problem description".to_owned(),
                tenant_id: f.tenant_id,
                at: Timestamp(10),
            }))
        );
        assert_eq!(f.status(TicketId(1)), TicketStatus::Open);
    }

    #[test]
    fn ticket_ids_are_sequential() {
        let mut f = fixture();
        assert_eq!(f.create_ticket(), TicketId(1));
        assert_eq!(f.create_ticket(), TicketId(2));
    }

    #[test]
    fn ticket_for_unknown_tenant_is_rejected() {
        let mut f = fixture();
        let command = CreateTicketCommand::new("Billing", "Wrong invoice", TenantId(99));
        assert_eq!(
            f.service.handle_create_ticket(command),
            Err(ServiceError::UnknownTenant(TenantId(99)))
        );
    }

    #[test]
    fn rejected_ticket_does_not_consume_id() {
        let mut f = fixture();
        let command = CreateTicketCommand::new("Billing", "  ", f.tenant_id);
        assert_eq!(
            f.service.handle_create_ticket(command),
            Err(ServiceError::Ticket(TicketError::EmptyDescription))
        );
        assert_eq!(f.create_ticket(), TicketId(1));
    }

    #[test]
    fn assign_agent_emits_event_and_updates_ticket() {
        let mut f = fixture();
        let ticket_id = f.create_ticket();
        f.clock.advance(60);
        assert_eq!(
            f.assign(ticket_id, f.agent_id),
            Ok(TicketEvent::AgentAssigned(AgentAssignedEvent {
                ticket_id,
                agent_id: f.agent_id,
                previous_agent_id: None,
                at: Timestamp(60),
            }))
        );
        let ticket = f.service.ticket(ticket_id).unwrap().unwrap();
        assert_eq!(ticket.status(), TicketStatus::InProgress);
        assert_eq!(ticket.assigned_agent(), Some(f.agent_id));
    }

    #[test]
    fn customer_cannot_be_assigned() {
        let mut f = fixture();
        let ticket_id = f.create_ticket();
        assert_eq!(
            f.assign(ticket_id, f.customer_id),
            Err(ServiceError::Ticket(TicketError::NotAnAgent {
                user_id: f.customer_id,
                role: Role::Customer
            }))
        );
        assert_eq!(f.status(ticket_id), TicketStatus::Open);
    }

    #[test]
    fn unknown_agent_or_ticket_is_rejected() {
        let mut f = fixture();
        let ticket_id = f.create_ticket();
        assert_eq!(
            f.assign(ticket_id, UserId(99)),
            Err(ServiceError::UnknownUser(UserId(99)))
        );
        assert_eq!(
            f.assign(TicketId(99), f.agent_id),
            Err(ServiceError::Ticket(TicketError::NotFound))
        );
    }

    #[test]
    fn close_ticket_emits_closed_event() {
        let mut f = fixture();
        let ticket_id = f.create_ticket();
        let command = CloseTicketCommand {
            ticket_id,
            reason: CloseReason::Resolved,
        };
        assert_eq!(
            f.service.handle_close_ticket(command),
            Ok(TicketEvent::Closed(TicketClosedEvent {
                ticket_id,
                reason: CloseReason::Resolved,
                at: Timestamp(0)
            }))
        );
        assert_eq!(f.status(ticket_id), TicketStatus::Closed);
        assert_eq!(
            f.service.handle_close_ticket(command),
            Err(ServiceError::Ticket(TicketError::Closed(ticket_id)))
        );
    }

    #[test]
    fn manual_close_for_inactivity_requires_timeout() {
        let mut f = fixture();
        let ticket_id = f.create_ticket();
        f.clock.advance(DAY);
        let command = CloseTicketCommand {
            ticket_id,
            reason: CloseReason::Inactivity,
        };
        assert_eq!(
            f.service.handle_close_ticket(command),
            Err(ServiceError::Ticket(TicketError::StillActive {
                idle_secs: DAY
            }))
        );
    }

    #[test]
    fn inactive_tickets_are_closed_automatically() {
        let mut f = fixture();
        let stale = f.create_ticket();
        let touched = f.create_ticket();
        f.clock.advance(3 * DAY);
        f.assign(touched, f.agent_id).unwrap();
        f.clock.advance(INACTIVITY_TIMEOUT_SECS - 3 * DAY);

        assert_eq!(
            f.service.close_inactive_tickets(),
            Ok(vec![TicketEvent::Closed(TicketClosedEvent {
                ticket_id: stale,
                reason: CloseReason::Inactivity,
                at: Timestamp(INACTIVITY_TIMEOUT_SECS),
            })])
        );
        assert_eq!(f.status(stale), TicketStatus::Closed);
        assert_eq!(f.status(touched), TicketStatus::InProgress);

        f.clock.advance(3 * DAY);
        let closed: Vec<_> = f
            .service
            .close_inactive_tickets()
            .unwrap()
            .iter()
            .map(TicketEvent::ticket_id)
            .collect();
        assert_eq!(closed, [touched]);
    }

    #[test]
    fn nothing_to_close_when_all_tickets_are_active() {
        let mut f = fixture();
        f.create_ticket();
        assert_eq!(f.service.close_inactive_tickets(), Ok(vec![]));
    }

    #[test]
    fn discount_follows_number_of_open_tickets() {
        let mut f = fixture();
        let tickets: Vec<_> = (0..3).map(|_| f.create_ticket()).collect();

        assert_eq!(
            f.apply_discount(),
            Ok(TenantEvent::DiscountApplied(DiscountAppliedEvent {
                tenant_id: f.tenant_id,
                previous_rate: DiscountRate::NONE,
                discount_rate: DiscountRate::for_open_tickets(3),
                open_tickets: 3,
            }))
        );
        let tenant = f.service.tenant(f.tenant_id).unwrap().unwrap();
        assert_eq!(tenant.discount_rate(), DiscountRate::for_open_tickets(3));

        f.service
            .handle_close_ticket(CloseTicketCommand {
                ticket_id: tickets[0],
                reason: CloseReason::Resolved,
            })
            .unwrap();
        let Ok(TenantEvent::DiscountApplied(event)) = f.apply_discount() else {
            panic!("discount goes down after closing a ticket");
        };
        assert_eq!(event.open_tickets, 2);
        assert_eq!(event.discount_rate, DiscountRate::NONE);
    }

    #[test]
    fn unchanged_discount_is_rejected() {
        let mut f = fixture();
        assert_eq!(
            f.apply_discount(),
            Err(ServiceError::Tenant(TenantError::DiscountUnchanged(
                DiscountRate::NONE
            )))
        );
    }

    #[test]
    fn discount_for_unknown_tenant_is_rejected() {
        let mut f = fixture();
        assert_eq!(
            f.service.handle_apply_discount(ApplyDiscountCommand {
                tenant_id: TenantId(99)
            }),
            Err(ServiceError::Tenant(TenantError::NotFound))
        );
    }

    #[test]
    fn other_tenants_tickets_do_not_affect_discount() {
        let mut f = fixture();
        let other = f
            .service
            .register_tenant(RegisterTenantCommand::new("Globex"))
            .unwrap()
            .tenant_id();
        for _ in 0..5 {
            f.service
                .handle_create_ticket(CreateTicketCommand::new("Billing", "Invoice", other))
                .unwrap();
        }
        assert!(matches!(
            f.apply_discount(),
            Err(ServiceError::Tenant(TenantError::DiscountUnchanged(_)))
        ));
    }

    #[test]
    fn registration_rejects_blank_names() {
        let mut f = fixture();
        assert_eq!(
            f.service.register_tenant(RegisterTenantCommand::new(" ")),
            Err(ServiceError::Tenant(TenantError::EmptyName))
        );
        assert!(matches!(
            f.service
                .register_user(RegisterUserCommand::new("", Role::Agent)),
            Err(ServiceError::User(_))
        ));
    }
}
