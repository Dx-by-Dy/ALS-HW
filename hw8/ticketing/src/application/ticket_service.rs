use std::num::NonZeroUsize;

use super::{
    clock::Clock,
    commands::{
        ApplyDiscountCommand, AssignAgentCommand, CloseTicketCommand, CreateTicketCommand,
        RegisterTenantCommand, RegisterUserCommand, ReopenTicketCommand,
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
    A: crate::core::event_sourced::EventSourced + Default + Clone + Send + Sync,
    A::Event: Clone + Send + Sync,
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

    pub async fn register_tenant(
        &mut self,
        command: RegisterTenantCommand,
    ) -> Result<TenantEvent, ServiceError> {
        let tenant_id = TenantId(self.next_tenant_id);
        let command = TenantCommand::Register {
            tenant_id,
            tenant_name: command.tenant_name,
        };
        let event = self.tenants.execute(tenant_id.into(), &command).await?;
        self.next_tenant_id += 1;
        Ok(event)
    }

    pub async fn register_user(
        &mut self,
        command: RegisterUserCommand,
    ) -> Result<UserEvent, ServiceError> {
        let user_id = UserId(self.next_user_id);
        let command = UserCommand::Register {
            user_id,
            user_name: command.user_name,
            role: command.role,
        };
        let event = self.users.execute(user_id.into(), &command).await?;
        self.next_user_id += 1;
        Ok(event)
    }

    pub async fn handle_create_ticket(
        &mut self,
        command: CreateTicketCommand,
    ) -> Result<TicketEvent, ServiceError> {
        if self.tenant(command.tenant_id).await?.is_none() {
            return Err(ServiceError::UnknownTenant(command.tenant_id));
        }
        let ticket_id = TicketId(self.next_ticket_id);
        let command = TicketCommand::Create {
            ticket_id,
            category: command.category,
            description: command.description,
            tenant_id: command.tenant_id,
            at: self.clock.now(),
        };
        let event = self.execute_ticket(ticket_id, command).await?;
        self.next_ticket_id += 1;
        Ok(event)
    }

    pub async fn handle_assign_agent(
        &mut self,
        command: AssignAgentCommand,
    ) -> Result<TicketEvent, ServiceError> {
        let agent = self
            .user(command.agent_id)
            .await?
            .ok_or(ServiceError::UnknownUser(command.agent_id))?;
        let at = self.clock.now();
        self.execute_ticket(command.ticket_id, TicketCommand::AssignAgent { agent, at })
            .await
    }

    pub async fn handle_close_ticket(
        &mut self,
        command: CloseTicketCommand,
    ) -> Result<TicketEvent, ServiceError> {
        let at = self.clock.now();
        let close = TicketCommand::Close {
            reason: command.reason,
            at,
        };
        self.execute_ticket(command.ticket_id, close).await
    }

    pub async fn handle_reopen_ticket(
        &mut self,
        command: ReopenTicketCommand,
    ) -> Result<TicketEvent, ServiceError> {
        let at = self.clock.now();
        self.execute_ticket(command.ticket_id, TicketCommand::Reopen { at })
            .await
    }

    /// Закрывает все тикеты, простоявшие без активности дольше таймаута.
    pub async fn close_inactive_tickets(&mut self) -> Result<Vec<TicketEvent>, ServiceError> {
        let now = self.clock.now();
        let mut events = Vec::new();
        for ticket_id in self.open_tickets.inactive_at(now) {
            let command = TicketCommand::Close {
                reason: CloseReason::Inactivity,
                at: now,
            };
            events.push(self.execute_ticket(ticket_id, command).await?);
        }
        Ok(events)
    }

    /// Пересчитывает скидку тенанта по текущему числу его открытых тикетов.
    pub async fn handle_apply_discount(
        &mut self,
        command: ApplyDiscountCommand,
    ) -> Result<TenantEvent, ServiceError> {
        let open_tickets = self.open_tickets.count_for_tenant(command.tenant_id);
        let command_for_tenant = TenantCommand::ApplyDiscount { open_tickets };
        Ok(self
            .tenants
            .execute(command.tenant_id.into(), &command_for_tenant)
            .await?)
    }

    pub async fn ticket(&self, ticket_id: TicketId) -> Result<Option<Ticket>, ServiceError> {
        Ok(self.tickets.load(ticket_id.into()).await?)
    }

    pub async fn tenant(&self, tenant_id: TenantId) -> Result<Option<Tenant>, ServiceError> {
        Ok(self.tenants.load(tenant_id.into()).await?)
    }

    pub async fn user(&self, user_id: UserId) -> Result<Option<User>, ServiceError> {
        Ok(self.users.load(user_id.into()).await?)
    }

    /// Все тикеты в порядке создания.
    pub async fn tickets(&self) -> Result<Vec<Ticket>, ServiceError> {
        let mut tickets = Vec::new();
        for id in 1..self.next_ticket_id {
            tickets.extend(self.ticket(TicketId(id)).await?);
        }
        Ok(tickets)
    }

    /// Все тенанты в порядке регистрации.
    pub async fn tenants(&self) -> Result<Vec<Tenant>, ServiceError> {
        let mut tenants = Vec::new();
        for id in 1..self.next_tenant_id {
            tenants.extend(self.tenant(TenantId(id)).await?);
        }
        Ok(tenants)
    }

    /// Все пользователи в порядке регистрации.
    pub async fn users(&self) -> Result<Vec<User>, ServiceError> {
        let mut users = Vec::new();
        for id in 1..self.next_user_id {
            users.extend(self.user(UserId(id)).await?);
        }
        Ok(users)
    }

    pub fn open_tickets_count(&self, tenant_id: TenantId) -> usize {
        self.open_tickets.count_for_tenant(tenant_id)
    }

    async fn execute_ticket(
        &mut self,
        ticket_id: TicketId,
        command: TicketCommand,
    ) -> Result<TicketEvent, ServiceError> {
        let event = self.tickets.execute(ticket_id.into(), &command).await?;
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
            TicketError, TicketReopenedEvent, TicketStatus,
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

    async fn fixture() -> Fixture {
        let clock = ManualClock::default();
        let mut service = TicketService::new(clock.clone());
        let tenant_id = service
            .register_tenant(RegisterTenantCommand::new("Acme"))
            .await
            .unwrap()
            .tenant_id();
        let agent_id = service
            .register_user(RegisterUserCommand::new("Alice", Role::Agent))
            .await
            .unwrap()
            .user_id();
        let customer_id = service
            .register_user(RegisterUserCommand::new("Bob", Role::Customer))
            .await
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
        async fn create_ticket(&mut self) -> TicketId {
            let command =
                CreateTicketCommand::new("Technical Issue", "Problem description", self.tenant_id);
            self.service
                .handle_create_ticket(command)
                .await
                .unwrap()
                .ticket_id()
        }

        async fn assign(
            &mut self,
            ticket_id: TicketId,
            agent_id: UserId,
        ) -> Result<TicketEvent, ServiceError> {
            self.service
                .handle_assign_agent(AssignAgentCommand {
                    ticket_id,
                    agent_id,
                })
                .await
        }

        async fn status(&self, ticket_id: TicketId) -> TicketStatus {
            self.service
                .ticket(ticket_id)
                .await
                .unwrap()
                .expect("ticket exists")
                .status()
        }

        async fn apply_discount(&mut self) -> Result<TenantEvent, ServiceError> {
            self.service
                .handle_apply_discount(ApplyDiscountCommand {
                    tenant_id: self.tenant_id,
                })
                .await
        }
    }

    #[tokio::test]
    async fn create_ticket_emits_created_event() {
        let mut f = fixture().await;
        f.clock.advance(10);
        let command =
            CreateTicketCommand::new("Technical Issue", "Problem description", f.tenant_id);
        assert_eq!(
            f.service.handle_create_ticket(command).await,
            Ok(TicketEvent::Created(TicketCreatedEvent {
                ticket_id: TicketId(1),
                category: "Technical Issue".to_owned(),
                description: "Problem description".to_owned(),
                tenant_id: f.tenant_id,
                at: Timestamp(10),
            }))
        );
        assert_eq!(f.status(TicketId(1)).await, TicketStatus::Open);
    }

    #[tokio::test]
    async fn ticket_ids_are_sequential() {
        let mut f = fixture().await;
        assert_eq!(f.create_ticket().await, TicketId(1));
        assert_eq!(f.create_ticket().await, TicketId(2));
    }

    #[tokio::test]
    async fn ticket_for_unknown_tenant_is_rejected() {
        let mut f = fixture().await;
        let command = CreateTicketCommand::new("Billing", "Wrong invoice", TenantId(99));
        assert_eq!(
            f.service.handle_create_ticket(command).await,
            Err(ServiceError::UnknownTenant(TenantId(99)))
        );
    }

    #[tokio::test]
    async fn rejected_ticket_does_not_consume_id() {
        let mut f = fixture().await;
        let command = CreateTicketCommand::new("Billing", "  ", f.tenant_id);
        assert_eq!(
            f.service.handle_create_ticket(command).await,
            Err(ServiceError::Ticket(TicketError::EmptyDescription))
        );
        assert_eq!(f.create_ticket().await, TicketId(1));
    }

    #[tokio::test]
    async fn assign_agent_emits_event_and_updates_ticket() {
        let mut f = fixture().await;
        let ticket_id = f.create_ticket().await;
        f.clock.advance(60);
        assert_eq!(
            f.assign(ticket_id, f.agent_id).await,
            Ok(TicketEvent::AgentAssigned(AgentAssignedEvent {
                ticket_id,
                agent_id: f.agent_id,
                previous_agent_id: None,
                at: Timestamp(60),
            }))
        );
        let ticket = f.service.ticket(ticket_id).await.unwrap().unwrap();
        assert_eq!(ticket.status(), TicketStatus::InProgress);
        assert_eq!(ticket.assigned_agent(), Some(f.agent_id));
    }

    #[tokio::test]
    async fn customer_cannot_be_assigned() {
        let mut f = fixture().await;
        let ticket_id = f.create_ticket().await;
        assert_eq!(
            f.assign(ticket_id, f.customer_id).await,
            Err(ServiceError::Ticket(TicketError::NotAnAgent {
                user_id: f.customer_id,
                role: Role::Customer
            }))
        );
        assert_eq!(f.status(ticket_id).await, TicketStatus::Open);
    }

    #[tokio::test]
    async fn unknown_agent_or_ticket_is_rejected() {
        let mut f = fixture().await;
        let ticket_id = f.create_ticket().await;
        assert_eq!(
            f.assign(ticket_id, UserId(99)).await,
            Err(ServiceError::UnknownUser(UserId(99)))
        );
        assert_eq!(
            f.assign(TicketId(99), f.agent_id).await,
            Err(ServiceError::Ticket(TicketError::NotFound))
        );
    }

    #[tokio::test]
    async fn close_ticket_emits_closed_event() {
        let mut f = fixture().await;
        let ticket_id = f.create_ticket().await;
        let command = CloseTicketCommand {
            ticket_id,
            reason: CloseReason::Resolved,
        };
        assert_eq!(
            f.service.handle_close_ticket(command).await,
            Ok(TicketEvent::Closed(TicketClosedEvent {
                ticket_id,
                reason: CloseReason::Resolved,
                at: Timestamp(0)
            }))
        );
        assert_eq!(f.status(ticket_id).await, TicketStatus::Closed);
        assert_eq!(
            f.service.handle_close_ticket(command).await,
            Err(ServiceError::Ticket(TicketError::Closed(ticket_id)))
        );
    }

    #[tokio::test]
    async fn reopen_ticket_emits_reopened_event() {
        let mut f = fixture().await;
        let ticket_id = f.create_ticket().await;
        let reopen = ReopenTicketCommand { ticket_id };
        assert_eq!(
            f.service.handle_reopen_ticket(reopen).await,
            Err(ServiceError::Ticket(TicketError::AlreadyOpen(ticket_id)))
        );

        f.service
            .handle_close_ticket(CloseTicketCommand {
                ticket_id,
                reason: CloseReason::Resolved,
            })
            .await
            .unwrap();
        f.clock.advance(60);
        assert_eq!(
            f.service.handle_reopen_ticket(reopen).await,
            Ok(TicketEvent::Reopened(TicketReopenedEvent {
                ticket_id,
                tenant_id: f.tenant_id,
                at: Timestamp(60),
            }))
        );
        assert_eq!(f.status(ticket_id).await, TicketStatus::Open);
        assert_eq!(f.service.open_tickets_count(f.tenant_id), 1);
    }

    #[tokio::test]
    async fn listings_return_everything_in_creation_order() {
        let mut f = fixture().await;
        let first = f.create_ticket().await;
        let second = f.create_ticket().await;
        let ids: Vec<_> = f
            .service
            .tickets()
            .await
            .unwrap()
            .iter()
            .map(Ticket::ticket_id)
            .collect();
        assert_eq!(ids, [first, second]);
        let users: Vec<_> = f
            .service
            .users()
            .await
            .unwrap()
            .iter()
            .map(User::user_id)
            .collect();
        assert_eq!(users, [f.agent_id, f.customer_id]);
        let tenants: Vec<_> = f
            .service
            .tenants()
            .await
            .unwrap()
            .iter()
            .map(Tenant::tenant_id)
            .collect();
        assert_eq!(tenants, [f.tenant_id]);
    }

    #[tokio::test]
    async fn manual_close_for_inactivity_requires_timeout() {
        let mut f = fixture().await;
        let ticket_id = f.create_ticket().await;
        f.clock.advance(DAY);
        let command = CloseTicketCommand {
            ticket_id,
            reason: CloseReason::Inactivity,
        };
        assert_eq!(
            f.service.handle_close_ticket(command).await,
            Err(ServiceError::Ticket(TicketError::StillActive {
                idle_secs: DAY
            }))
        );
    }

    #[tokio::test]
    async fn inactive_tickets_are_closed_automatically() {
        let mut f = fixture().await;
        let stale = f.create_ticket().await;
        let touched = f.create_ticket().await;
        f.clock.advance(3 * DAY);
        f.assign(touched, f.agent_id).await.unwrap();
        f.clock.advance(INACTIVITY_TIMEOUT_SECS - 3 * DAY);

        assert_eq!(
            f.service.close_inactive_tickets().await,
            Ok(vec![TicketEvent::Closed(TicketClosedEvent {
                ticket_id: stale,
                reason: CloseReason::Inactivity,
                at: Timestamp(INACTIVITY_TIMEOUT_SECS),
            })])
        );
        assert_eq!(f.status(stale).await, TicketStatus::Closed);
        assert_eq!(f.status(touched).await, TicketStatus::InProgress);

        f.clock.advance(3 * DAY);
        let closed: Vec<_> = f
            .service
            .close_inactive_tickets()
            .await
            .unwrap()
            .iter()
            .map(TicketEvent::ticket_id)
            .collect();
        assert_eq!(closed, [touched]);
    }

    #[tokio::test]
    async fn nothing_to_close_when_all_tickets_are_active() {
        let mut f = fixture().await;
        f.create_ticket().await;
        assert_eq!(f.service.close_inactive_tickets().await, Ok(vec![]));
    }

    #[tokio::test]
    async fn discount_follows_number_of_open_tickets() {
        let mut f = fixture().await;
        let mut tickets = Vec::new();
        for _ in 0..6 {
            tickets.push(f.create_ticket().await);
        }

        assert_eq!(
            f.apply_discount().await,
            Ok(TenantEvent::DiscountApplied(DiscountAppliedEvent {
                tenant_id: f.tenant_id,
                previous_rate: DiscountRate::NONE,
                discount_rate: DiscountRate::STANDARD,
                open_tickets: 6,
            }))
        );
        let tenant = f.service.tenant(f.tenant_id).await.unwrap().unwrap();
        assert_eq!(tenant.discount_rate(), DiscountRate::STANDARD);

        f.service
            .handle_close_ticket(CloseTicketCommand {
                ticket_id: tickets[0],
                reason: CloseReason::Resolved,
            })
            .await
            .unwrap();
        let Ok(TenantEvent::DiscountApplied(event)) = f.apply_discount().await else {
            panic!("discount goes down after closing a ticket");
        };
        assert_eq!(event.open_tickets, 5);
        assert_eq!(event.discount_rate, DiscountRate::NONE);

        f.service
            .handle_reopen_ticket(ReopenTicketCommand {
                ticket_id: tickets[0],
            })
            .await
            .unwrap();
        let Ok(TenantEvent::DiscountApplied(event)) = f.apply_discount().await else {
            panic!("reopened ticket counts again");
        };
        assert_eq!(event.open_tickets, 6);
        assert_eq!(event.discount_rate, DiscountRate::STANDARD);
    }

    #[tokio::test]
    async fn unchanged_discount_is_rejected() {
        let mut f = fixture().await;
        assert_eq!(
            f.apply_discount().await,
            Err(ServiceError::Tenant(TenantError::DiscountUnchanged(
                DiscountRate::NONE
            )))
        );
    }

    #[tokio::test]
    async fn discount_for_unknown_tenant_is_rejected() {
        let mut f = fixture().await;
        assert_eq!(
            f.service
                .handle_apply_discount(ApplyDiscountCommand {
                    tenant_id: TenantId(99)
                })
                .await,
            Err(ServiceError::Tenant(TenantError::NotFound))
        );
    }

    #[tokio::test]
    async fn other_tenants_tickets_do_not_affect_discount() {
        let mut f = fixture().await;
        let other = f
            .service
            .register_tenant(RegisterTenantCommand::new("Globex"))
            .await
            .unwrap()
            .tenant_id();
        for _ in 0..=DiscountRate::THRESHOLD {
            f.service
                .handle_create_ticket(CreateTicketCommand::new("Billing", "Invoice", other))
                .await
                .unwrap();
        }
        assert!(matches!(
            f.apply_discount().await,
            Err(ServiceError::Tenant(TenantError::DiscountUnchanged(_)))
        ));
    }

    #[tokio::test]
    async fn registration_rejects_blank_names() {
        let mut f = fixture().await;
        assert_eq!(
            f.service
                .register_tenant(RegisterTenantCommand::new(" "))
                .await,
            Err(ServiceError::Tenant(TenantError::EmptyName))
        );
        assert!(matches!(
            f.service
                .register_user(RegisterUserCommand::new("", Role::Agent))
                .await,
            Err(ServiceError::User(_))
        ));
    }
}
