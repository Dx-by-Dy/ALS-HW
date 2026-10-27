use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::fmt;

use framework::cqrs::envelope::{Envelope, Version};
use ticketing::domain::{
    DomainEvent,
    tenant::{DiscountRate, TenantEvent, TenantId},
    ticket::{TicketEvent, TicketId, TicketStatus},
    time::Timestamp,
    user::{UserEvent, UserId},
};

use crate::queries::ListTickets;
use crate::views::{Changes, TenantView, TicketView, UserView};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionError {
    /// Пропущены события: модель на версии `current`, а пришло событие `got`.
    Gap {
        current: Version,
        got: Version,
    },
    UnknownTicket(TicketId),
    UnknownTenant(TenantId),
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gap { current, got } => {
                write!(f, "missed events: read model is at {current}, got {got}")
            }
            Self::UnknownTicket(id) => write!(f, "event refers to unknown {id}"),
            Self::UnknownTenant(id) => write!(f, "event refers to unknown {id}"),
        }
    }
}

impl Error for ProjectionError {}

/// Запись read-модели — ключ в индексе изменений.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Ticket(TicketId),
    Tenant(TenantId),
    User(UserId),
}

/// Содержимое read-модели: записи и индексы под запросы.
///
/// Всё обновляется синхронно в [`apply`](Self::apply), поэтому индексы всегда согласованы
/// с записями. Асинхронность и разделение доступа — забота [`ReadModel`](crate::ReadModel).
#[derive(Debug, Default)]
pub(crate) struct State {
    version: Version,
    tickets: BTreeMap<TicketId, TicketView>,
    tenants: BTreeMap<TenantId, TenantView>,
    users: BTreeMap<UserId, UserView>,
    /// Незакрытые тикеты каждого агента.
    active_by_agent: HashMap<UserId, BTreeSet<TicketId>>,
    /// Все тикеты каждого тенанта.
    by_tenant: HashMap<TenantId, BTreeSet<TicketId>>,
    /// Закрытые тикеты по времени закрытия — для запросов по диапазону времени.
    closed: BTreeSet<(Timestamp, TicketId)>,
    /// Каждая запись под версией своего последнего изменения — для запроса изменений.
    changed: BTreeSet<(Version, Key)>,
}

impl State {
    pub fn version(&self) -> Version {
        self.version
    }

    /// Применяет событие следующей версии.
    ///
    /// Уже применённое событие пропускается, поэтому повторная доставка безопасна.
    /// Пропуск версии или ссылка на неизвестную запись — ошибка, и модель не меняется.
    pub fn apply(&mut self, envelope: &Envelope<DomainEvent>) -> Result<(), ProjectionError> {
        let version = envelope.version;
        if version <= self.version {
            return Ok(());
        }
        if version != self.version.next() {
            return Err(ProjectionError::Gap {
                current: self.version,
                got: version,
            });
        }
        match &envelope.event {
            DomainEvent::Ticket(event) => self.apply_ticket(event, version)?,
            DomainEvent::Tenant(event) => self.apply_tenant(event, version)?,
            DomainEvent::User(event) => self.apply_user(event, version),
        }
        self.version = version;
        Ok(())
    }

    // Каждый обработчик сначала находит все нужные записи и только потом меняет их:
    // так ошибка не оставляет модель наполовину обновлённой.

    fn apply_ticket(
        &mut self,
        event: &TicketEvent,
        version: Version,
    ) -> Result<(), ProjectionError> {
        match event {
            TicketEvent::Created(e) => {
                let tenant = self
                    .tenants
                    .get_mut(&e.tenant_id)
                    .ok_or(ProjectionError::UnknownTenant(e.tenant_id))?;
                tenant.open_tickets += 1;
                touch(
                    &mut self.changed,
                    Key::Tenant(e.tenant_id),
                    &mut tenant.version,
                    version,
                );
                self.tickets.insert(
                    e.ticket_id,
                    TicketView {
                        ticket_id: e.ticket_id,
                        category: e.category.clone(),
                        description: e.description.clone(),
                        status: TicketStatus::Open,
                        assigned_agent: None,
                        tenant_id: e.tenant_id,
                        created_at: e.at,
                        closed_at: None,
                        version,
                    },
                );
                self.changed.insert((version, Key::Ticket(e.ticket_id)));
                self.by_tenant
                    .entry(e.tenant_id)
                    .or_default()
                    .insert(e.ticket_id);
            }
            TicketEvent::AgentAssigned(e) => {
                let ticket = self
                    .tickets
                    .get_mut(&e.ticket_id)
                    .ok_or(ProjectionError::UnknownTicket(e.ticket_id))?;
                if let Some(previous) = ticket.assigned_agent {
                    remove_from(&mut self.active_by_agent, previous, e.ticket_id);
                }
                self.active_by_agent
                    .entry(e.agent_id)
                    .or_default()
                    .insert(e.ticket_id);
                ticket.assigned_agent = Some(e.agent_id);
                ticket.status = TicketStatus::InProgress;
                touch(
                    &mut self.changed,
                    Key::Ticket(e.ticket_id),
                    &mut ticket.version,
                    version,
                );
            }
            TicketEvent::Closed(e) => {
                let ticket = self
                    .tickets
                    .get_mut(&e.ticket_id)
                    .ok_or(ProjectionError::UnknownTicket(e.ticket_id))?;
                let tenant = self
                    .tenants
                    .get_mut(&ticket.tenant_id)
                    .ok_or(ProjectionError::UnknownTenant(ticket.tenant_id))?;
                if let Some(agent) = ticket.assigned_agent {
                    remove_from(&mut self.active_by_agent, agent, e.ticket_id);
                }
                self.closed.insert((e.at, e.ticket_id));
                ticket.status = TicketStatus::Closed;
                ticket.closed_at = Some(e.at);
                touch(
                    &mut self.changed,
                    Key::Ticket(e.ticket_id),
                    &mut ticket.version,
                    version,
                );
                tenant.open_tickets = tenant.open_tickets.saturating_sub(1);
                touch(
                    &mut self.changed,
                    Key::Tenant(tenant.tenant_id),
                    &mut tenant.version,
                    version,
                );
            }
            TicketEvent::Reopened(e) => {
                let ticket = self
                    .tickets
                    .get_mut(&e.ticket_id)
                    .ok_or(ProjectionError::UnknownTicket(e.ticket_id))?;
                let tenant = self
                    .tenants
                    .get_mut(&ticket.tenant_id)
                    .ok_or(ProjectionError::UnknownTenant(ticket.tenant_id))?;
                if let Some(closed_at) = ticket.closed_at.take() {
                    self.closed.remove(&(closed_at, e.ticket_id));
                }
                // Как и агрегат, открытый заново тикет начинает без агента.
                ticket.status = TicketStatus::Open;
                ticket.assigned_agent = None;
                touch(
                    &mut self.changed,
                    Key::Ticket(e.ticket_id),
                    &mut ticket.version,
                    version,
                );
                tenant.open_tickets += 1;
                touch(
                    &mut self.changed,
                    Key::Tenant(tenant.tenant_id),
                    &mut tenant.version,
                    version,
                );
            }
        }
        Ok(())
    }

    fn apply_tenant(
        &mut self,
        event: &TenantEvent,
        version: Version,
    ) -> Result<(), ProjectionError> {
        match event {
            TenantEvent::Registered(e) => {
                self.tenants.insert(
                    e.tenant_id,
                    TenantView {
                        tenant_id: e.tenant_id,
                        tenant_name: e.tenant_name.clone(),
                        discount_rate: DiscountRate::NONE,
                        open_tickets: 0,
                        version,
                    },
                );
                self.changed.insert((version, Key::Tenant(e.tenant_id)));
            }
            TenantEvent::DiscountApplied(e) => {
                let tenant = self
                    .tenants
                    .get_mut(&e.tenant_id)
                    .ok_or(ProjectionError::UnknownTenant(e.tenant_id))?;
                tenant.discount_rate = e.discount_rate;
                touch(
                    &mut self.changed,
                    Key::Tenant(e.tenant_id),
                    &mut tenant.version,
                    version,
                );
            }
        }
        Ok(())
    }

    fn apply_user(&mut self, event: &UserEvent, version: Version) {
        match event {
            UserEvent::Registered(e) => {
                self.users.insert(
                    e.user_id,
                    UserView {
                        user_id: e.user_id,
                        user_name: e.user_name.clone(),
                        role: e.role,
                        version,
                    },
                );
                self.changed.insert((version, Key::User(e.user_id)));
            }
        }
    }

    pub fn ticket(&self, ticket_id: TicketId) -> Option<TicketView> {
        self.tickets.get(&ticket_id).cloned()
    }

    pub fn tickets(&self, filter: ListTickets) -> Vec<TicketView> {
        let matches = |t: &&TicketView| filter.status.is_none_or(|status| t.status == status);
        match filter.tenant_id {
            Some(tenant_id) => self
                .tickets_of_tenant(tenant_id)
                .filter(matches)
                .cloned()
                .collect(),
            None => self.tickets.values().filter(matches).cloned().collect(),
        }
    }

    pub fn active_tickets_of_agent(&self, agent_id: UserId) -> Vec<TicketView> {
        let ids = self.active_by_agent.get(&agent_id).into_iter().flatten();
        ids.map(|id| self.tickets[id].clone()).collect()
    }

    pub fn unassigned_tickets_of_tenant(&self, tenant_id: TenantId) -> Vec<TicketView> {
        self.tickets_of_tenant(tenant_id)
            .filter(|t| t.status == TicketStatus::Open && t.assigned_agent.is_none())
            .cloned()
            .collect()
    }

    pub fn tickets_closed_since(&self, since: Timestamp) -> Vec<TicketView> {
        self.closed
            .range((since, TicketId(0))..)
            .map(|(_, id)| self.tickets[id].clone())
            .collect()
    }

    pub fn tenant(&self, tenant_id: TenantId) -> Option<TenantView> {
        self.tenants.get(&tenant_id).cloned()
    }

    pub fn tenants(&self) -> Vec<TenantView> {
        self.tenants.values().cloned().collect()
    }

    pub fn users(&self) -> Vec<UserView> {
        self.users.values().cloned().collect()
    }

    pub fn changes_since(&self, after: Version) -> Changes {
        let mut changes = Changes {
            version: self.version,
            ..Changes::default()
        };
        for (_, key) in self
            .changed
            .range((after.next(), Key::Ticket(TicketId(0)))..)
        {
            match key {
                Key::Ticket(id) => changes.tickets.push(self.tickets[id].clone()),
                Key::Tenant(id) => changes.tenants.push(self.tenants[id].clone()),
                Key::User(id) => changes.users.push(self.users[id].clone()),
            }
        }
        changes
    }

    fn tickets_of_tenant(&self, tenant_id: TenantId) -> impl Iterator<Item = &TicketView> {
        let ids = self.by_tenant.get(&tenant_id).into_iter().flatten();
        ids.map(|id| &self.tickets[id])
    }
}

/// Переносит запись в индексе изменений на новую версию.
fn touch(
    changed: &mut BTreeSet<(Version, Key)>,
    key: Key,
    record_version: &mut Version,
    version: Version,
) {
    changed.remove(&(*record_version, key));
    changed.insert((version, key));
    *record_version = version;
}

fn remove_from(index: &mut HashMap<UserId, BTreeSet<TicketId>>, agent: UserId, ticket: TicketId) {
    if let Some(tickets) = index.get_mut(&agent) {
        tickets.remove(&ticket);
        if tickets.is_empty() {
            index.remove(&agent);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ticketing::domain::{
        tenant::{DiscountAppliedEvent, TenantRegisteredEvent},
        ticket::{
            AgentAssignedEvent, CloseReason, TicketClosedEvent, TicketCreatedEvent,
            TicketReopenedEvent,
        },
        user::{Role, UserRegisteredEvent},
    };

    pub const ACME: TenantId = TenantId(1);
    pub const GLOBEX: TenantId = TenantId(2);
    pub const ALICE: UserId = UserId(1);
    pub const CAROL: UserId = UserId(2);

    pub fn tenant(tenant_id: TenantId, name: &str) -> DomainEvent {
        TenantEvent::Registered(TenantRegisteredEvent {
            tenant_id,
            tenant_name: name.to_owned(),
        })
        .into()
    }

    pub fn agent(user_id: UserId, name: &str) -> DomainEvent {
        UserEvent::Registered(UserRegisteredEvent {
            user_id,
            user_name: name.to_owned(),
            role: Role::Agent,
        })
        .into()
    }

    pub fn created(ticket: usize, tenant_id: TenantId, at: u64) -> DomainEvent {
        TicketEvent::Created(TicketCreatedEvent {
            ticket_id: TicketId(ticket),
            category: "Billing".to_owned(),
            description: format!("Ticket {ticket}"),
            tenant_id,
            at: Timestamp(at),
        })
        .into()
    }

    pub fn assigned(ticket: usize, agent_id: UserId, previous: Option<UserId>) -> DomainEvent {
        TicketEvent::AgentAssigned(AgentAssignedEvent {
            ticket_id: TicketId(ticket),
            agent_id,
            previous_agent_id: previous,
            at: Timestamp(0),
        })
        .into()
    }

    pub fn closed(ticket: usize, at: u64) -> DomainEvent {
        TicketEvent::Closed(TicketClosedEvent {
            ticket_id: TicketId(ticket),
            reason: CloseReason::Resolved,
            at: Timestamp(at),
        })
        .into()
    }

    pub fn reopened(ticket: usize, tenant_id: TenantId) -> DomainEvent {
        TicketEvent::Reopened(TicketReopenedEvent {
            ticket_id: TicketId(ticket),
            tenant_id,
            at: Timestamp(0),
        })
        .into()
    }

    pub fn discount(tenant_id: TenantId, rate: DiscountRate) -> DomainEvent {
        TenantEvent::DiscountApplied(DiscountAppliedEvent {
            tenant_id,
            previous_rate: DiscountRate::NONE,
            discount_rate: rate,
            open_tickets: 0,
        })
        .into()
    }

    /// Состояние после событий с версиями 1, 2, …
    fn state_after(events: impl IntoIterator<Item = DomainEvent>) -> State {
        let mut state = State::default();
        for event in events {
            let envelope = Envelope {
                version: state.version().next(),
                event,
            };
            state.apply(&envelope).unwrap();
        }
        state
    }

    /// Acme и Globex (v1, v2), агенты Alice и Carol (v3, v4).
    fn registrations() -> Vec<DomainEvent> {
        vec![
            tenant(ACME, "Acme"),
            tenant(GLOBEX, "Globex"),
            agent(ALICE, "Alice"),
            agent(CAROL, "Carol"),
        ]
    }

    fn with(events: impl IntoIterator<Item = DomainEvent>) -> State {
        state_after(registrations().into_iter().chain(events))
    }

    fn ids(tickets: &[TicketView]) -> Vec<usize> {
        tickets.iter().map(|t| t.ticket_id.0).collect()
    }

    fn open_tickets(state: &State, tenant_id: TenantId) -> usize {
        state.tenant(tenant_id).unwrap().open_tickets
    }

    #[test]
    fn registrations_create_tenant_and_user_views() {
        let state = with([]);
        assert_eq!(
            state.tenant(ACME),
            Some(TenantView {
                tenant_id: ACME,
                tenant_name: "Acme".to_owned(),
                discount_rate: DiscountRate::NONE,
                open_tickets: 0,
                version: Version(1),
            })
        );
        let users = state.users();
        assert_eq!(users.len(), 2);
        assert_eq!(users[1].user_name, "Carol");
        assert_eq!(users[1].version, Version(4));
    }

    #[test]
    fn created_ticket_is_open_and_counts_for_tenant() {
        let state = with([created(1, ACME, 10)]);
        assert_eq!(
            state.ticket(TicketId(1)),
            Some(TicketView {
                ticket_id: TicketId(1),
                category: "Billing".to_owned(),
                description: "Ticket 1".to_owned(),
                status: TicketStatus::Open,
                assigned_agent: None,
                tenant_id: ACME,
                created_at: Timestamp(10),
                closed_at: None,
                version: Version(5),
            })
        );
        assert_eq!(open_tickets(&state, ACME), 1);
        assert_eq!(open_tickets(&state, GLOBEX), 0);
        assert_eq!(state.tenant(ACME).unwrap().version, Version(5));
    }

    #[test]
    fn assignment_sets_agent_and_moves_ticket_between_agents() {
        let state = with([created(1, ACME, 0), assigned(1, ALICE, None)]);
        let ticket = state.ticket(TicketId(1)).unwrap();
        assert_eq!(ticket.assigned_agent, Some(ALICE));
        assert_eq!(ticket.status, TicketStatus::InProgress);
        assert_eq!(ids(&state.active_tickets_of_agent(ALICE)), [1]);

        let state = with([
            created(1, ACME, 0),
            assigned(1, ALICE, None),
            assigned(1, CAROL, Some(ALICE)),
        ]);
        assert!(state.active_tickets_of_agent(ALICE).is_empty());
        assert_eq!(ids(&state.active_tickets_of_agent(CAROL)), [1]);
    }

    #[test]
    fn closing_records_time_and_frees_agent_and_tenant() {
        let state = with([created(1, ACME, 0), assigned(1, ALICE, None), closed(1, 50)]);
        let ticket = state.ticket(TicketId(1)).unwrap();
        assert_eq!(ticket.status, TicketStatus::Closed);
        assert_eq!(ticket.closed_at, Some(Timestamp(50)));
        assert_eq!(ticket.assigned_agent, Some(ALICE));
        assert!(state.active_tickets_of_agent(ALICE).is_empty());
        assert_eq!(open_tickets(&state, ACME), 0);
    }

    #[test]
    fn reopened_ticket_is_open_unassigned_and_counts_again() {
        let state = with([
            created(1, ACME, 0),
            assigned(1, ALICE, None),
            closed(1, 50),
            reopened(1, ACME),
        ]);
        let ticket = state.ticket(TicketId(1)).unwrap();
        assert_eq!(ticket.status, TicketStatus::Open);
        assert_eq!(ticket.assigned_agent, None);
        assert_eq!(ticket.closed_at, None);
        assert!(state.tickets_closed_since(Timestamp(0)).is_empty());
        assert_eq!(open_tickets(&state, ACME), 1);
    }

    #[test]
    fn discount_applied_updates_tenant() {
        let state = with([discount(ACME, DiscountRate::STANDARD)]);
        let tenant = state.tenant(ACME).unwrap();
        assert_eq!(tenant.discount_rate, DiscountRate::STANDARD);
        assert_eq!(tenant.version, Version(5));
    }

    #[test]
    fn active_tickets_of_agent_skip_closed_and_others() {
        let state = with([
            created(1, ACME, 0),
            created(2, GLOBEX, 0),
            created(3, ACME, 0),
            assigned(1, ALICE, None),
            assigned(2, ALICE, None),
            assigned(3, CAROL, None),
            closed(2, 0),
        ]);
        assert_eq!(ids(&state.active_tickets_of_agent(ALICE)), [1]);
        assert_eq!(ids(&state.active_tickets_of_agent(CAROL)), [3]);
        assert!(state.active_tickets_of_agent(UserId(99)).is_empty());
    }

    #[test]
    fn unassigned_tickets_are_open_without_agent_of_that_tenant() {
        let state = with([
            created(1, ACME, 0),
            created(2, ACME, 0),
            created(3, ACME, 0),
            created(4, GLOBEX, 0),
            assigned(2, ALICE, None),
            closed(3, 0),
        ]);
        assert_eq!(ids(&state.unassigned_tickets_of_tenant(ACME)), [1]);
        assert_eq!(ids(&state.unassigned_tickets_of_tenant(GLOBEX)), [4]);
    }

    #[test]
    fn closed_since_includes_boundary_and_orders_by_closing_time() {
        let state = with([
            created(1, ACME, 0),
            created(2, ACME, 0),
            created(3, ACME, 0),
            closed(1, 300),
            closed(2, 100),
            closed(3, 200),
        ]);
        assert_eq!(ids(&state.tickets_closed_since(Timestamp(200))), [3, 1]);
        assert_eq!(ids(&state.tickets_closed_since(Timestamp(0))), [2, 3, 1]);
        assert!(state.tickets_closed_since(Timestamp(301)).is_empty());
    }

    #[test]
    fn list_tickets_filters_by_status_and_tenant() {
        let state = with([
            created(1, ACME, 0),
            created(2, GLOBEX, 0),
            created(3, ACME, 0),
            closed(3, 0),
        ]);
        assert_eq!(ids(&state.tickets(ListTickets::default())), [1, 2, 3]);
        let acme = ListTickets {
            tenant_id: Some(ACME),
            ..ListTickets::default()
        };
        assert_eq!(ids(&state.tickets(acme)), [1, 3]);
        let open_acme = ListTickets {
            status: Some(TicketStatus::Open),
            tenant_id: Some(ACME),
        };
        assert_eq!(ids(&state.tickets(open_acme)), [1]);
    }

    #[test]
    fn changes_since_return_each_record_once_in_latest_state() {
        let state = with([created(1, ACME, 0), created(2, ACME, 0), closed(1, 0)]);
        // v5: тикет 1 (и Acme), v6: тикет 2 (и Acme), v7: тикет 1 закрыт (и Acme).
        let changes = state.changes_since(Version(5));
        assert_eq!(changes.version, Version(7));
        assert_eq!(ids(&changes.tickets), [2, 1]);
        assert_eq!(changes.tickets[1].status, TicketStatus::Closed);
        assert_eq!(changes.tenants.len(), 1);
        assert_eq!(changes.tenants[0].open_tickets, 1);
        assert!(changes.users.is_empty());

        let everything = state.changes_since(Version(0));
        assert_eq!(
            (
                everything.tickets.len(),
                everything.tenants.len(),
                everything.users.len()
            ),
            (2, 2, 2)
        );
        assert_eq!(
            state.changes_since(Version(7)),
            Changes {
                version: Version(7),
                ..Changes::default()
            }
        );
    }

    #[test]
    fn redelivered_event_is_ignored() {
        let mut state = with([created(1, ACME, 0)]);
        let again = Envelope {
            version: Version(5),
            event: created(1, ACME, 0),
        };
        assert_eq!(state.apply(&again), Ok(()));
        assert_eq!(open_tickets(&state, ACME), 1);
    }

    #[test]
    fn gap_in_versions_is_an_error() {
        let mut state = with([]);
        let skipped = Envelope {
            version: Version(6),
            event: created(1, ACME, 0),
        };
        assert_eq!(
            state.apply(&skipped),
            Err(ProjectionError::Gap {
                current: Version(4),
                got: Version(6)
            })
        );
        assert_eq!(state.version(), Version(4));
    }

    #[test]
    fn event_for_unknown_record_changes_nothing() {
        let mut state = with([]);
        let orphan = Envelope {
            version: Version(5),
            event: closed(9, 0),
        };
        assert_eq!(
            state.apply(&orphan),
            Err(ProjectionError::UnknownTicket(TicketId(9)))
        );
        assert_eq!(state.version(), Version(4));
        assert!(state.changes_since(Version(4)).tenants.is_empty());
    }
}
