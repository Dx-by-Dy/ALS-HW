mod command;
mod error;
mod events;

use std::fmt;

pub use command::{CloseReason, ReminderKind, ResourceAvailability, TicketCommand};
pub use error::TicketError;
pub use events::{
    AgentAssignedEvent, HandedToPartnerEvent, PartnerProgressReportedEvent, ReminderSentEvent,
    ResourcesCheckedEvent, TicketClosedEvent, TicketCreatedEvent, TicketEvent, TicketReopenedEvent,
};
/// Статус и правила переходов — из Shared Kernel: их же соблюдает CRM.
pub use kernel::status::TicketStatus;

use crate::domain::{
    partner::PartnerId,
    tenant::TenantId,
    time::{DAY_SECS, Timestamp},
    user::{Role, User, UserId},
};
use framework::event_sourcing::{event_store::StreamId, lifecycle::Lifecycle};
use kernel::sla::{CustomerTier, Sla};

/// Сколько тикет может простоять без активности, прежде чем его закроют: 10 дней.
pub const INACTIVITY_TIMEOUT_SECS: u64 = 10 * DAY_SECS;

/// Когда напомнить о простаивающем тикете: через 3 дня без активности.
pub const STALE_REMINDER_AFTER_SECS: u64 = 3 * DAY_SECS;

/// Когда предупредить о скором закрытии: за день до истечения таймаута.
pub const CLOSING_SOON_REMINDER_AFTER_SECS: u64 = INACTIVITY_TIMEOUT_SECS - DAY_SECS;

/// Истёк ли таймаут неактивности к моменту `now`.
pub fn inactivity_expired(last_activity: Timestamp, now: Timestamp) -> bool {
    now.secs_since(last_activity) >= INACTIVITY_TIMEOUT_SECS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TicketId(pub usize);

impl From<TicketId> for StreamId {
    fn from(id: TicketId) -> Self {
        StreamId(id.0)
    }
}

impl fmt::Display for TicketId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ticket-{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    ticket_id: TicketId,
    category: String,
    description: String,
    status: TicketStatus,
    assigned_agent: Option<UserId>,
    tenant: TenantId,
    tier: CustomerTier,
    /// SLA текущего захода: от создания или последнего повторного открытия.
    sla: Sla,
    /// Время последнего изменения: от него отсчитывается неактивность.
    last_activity: Timestamp,
    /// Самое позднее напоминание, отправленное с момента последней активности.
    last_reminder: Option<ReminderKind>,
    /// Ответ ERP; `None`, пока ресурсы не проверены.
    resources: Option<ResourceAvailability>,
    /// Партнёр, которому передан тикет.
    partner: Option<PartnerId>,
}

impl Ticket {
    /// Решение о создании тикета. SLA начинает отсчёт с момента создания.
    pub fn create(
        ticket_id: TicketId,
        category: &str,
        description: &str,
        tenant_id: TenantId,
        tier: CustomerTier,
        at: Timestamp,
    ) -> Result<TicketEvent, TicketError> {
        let description = description.trim();
        if description.is_empty() {
            return Err(TicketError::EmptyDescription);
        }
        let category = category.trim();
        if category.is_empty() {
            return Err(TicketError::EmptyCategory);
        }
        Ok(TicketEvent::Created(TicketCreatedEvent {
            ticket_id,
            category: category.to_owned(),
            description: description.to_owned(),
            tenant_id,
            tier,
            sla: Sla::for_tier(tier, at),
            at,
        }))
    }

    /// Решение о назначении агента. Повторное назначение другого агента разрешено.
    pub fn assign_agent(&self, agent: &User, at: Timestamp) -> Result<TicketEvent, TicketError> {
        self.ensure_can_become(TicketStatus::InProgress)?;
        if agent.role() != Role::Agent {
            return Err(TicketError::NotAnAgent {
                user_id: agent.user_id(),
                role: agent.role(),
            });
        }
        if self.assigned_agent == Some(agent.user_id()) {
            return Err(TicketError::AlreadyAssigned(agent.user_id()));
        }
        Ok(TicketEvent::AgentAssigned(AgentAssignedEvent {
            ticket_id: self.ticket_id,
            agent_id: agent.user_id(),
            previous_agent_id: self.assigned_agent,
            at,
        }))
    }

    /// Решение о закрытии. По неактивности закрыть можно, только если таймаут истёк.
    pub fn close(&self, reason: CloseReason, at: Timestamp) -> Result<TicketEvent, TicketError> {
        self.ensure_can_become(TicketStatus::Closed)?;
        if reason == CloseReason::Inactivity && !inactivity_expired(self.last_activity, at) {
            return Err(TicketError::StillActive {
                idle_secs: at.secs_since(self.last_activity),
            });
        }
        Ok(TicketEvent::Closed(TicketClosedEvent {
            ticket_id: self.ticket_id,
            tenant_id: self.tenant,
            agent_id: self.assigned_agent,
            reason,
            sla_met: !self.sla.is_overdue(at),
            at,
        }))
    }

    /// Решение об открытии закрытого тикета. Он начинает жизнь заново: без агента,
    /// без партнёра, с новым SLA и новым отсчётом неактивности.
    pub fn reopen(&self, at: Timestamp) -> Result<TicketEvent, TicketError> {
        self.ensure_can_become(TicketStatus::Open)?;
        Ok(TicketEvent::Reopened(TicketReopenedEvent {
            ticket_id: self.ticket_id,
            tenant_id: self.tenant,
            category: self.category.clone(),
            sla: Sla::for_tier(self.tier, at),
            at,
        }))
    }

    /// Решение о напоминании. Срок и повторы проверяет агрегат: за то время, пока сработавший
    /// дедлайн дошёл сюда, в тикете могла появиться активность.
    pub fn send_reminder(
        &self,
        kind: ReminderKind,
        at: Timestamp,
    ) -> Result<TicketEvent, TicketError> {
        self.ensure_open()?;
        let idle_secs = at.secs_since(self.last_activity);
        if idle_secs < kind.due_after_secs() {
            return Err(TicketError::ReminderNotDue { kind, idle_secs });
        }
        if self.last_reminder >= Some(kind) {
            return Err(TicketError::ReminderAlreadySent(kind));
        }
        Ok(TicketEvent::ReminderSent(ReminderSentEvent {
            ticket_id: self.ticket_id,
            agent_id: self.assigned_agent,
            kind,
            at,
        }))
    }

    /// Решение о записи ответа ERP. Ресурсы проверяют один раз за заход: повторный ответ
    /// означает, что проверку повторили, и его нельзя молча принять.
    pub fn record_resource_check(
        &self,
        availability: ResourceAvailability,
        at: Timestamp,
    ) -> Result<TicketEvent, TicketError> {
        self.ensure_open()?;
        if self.resources.is_some() {
            return Err(TicketError::ResourcesAlreadyChecked(self.ticket_id));
        }
        Ok(TicketEvent::ResourcesChecked(ResourcesCheckedEvent {
            ticket_id: self.ticket_id,
            availability,
            at,
        }))
    }

    /// Решение о передаче партнёру: только если ERP сообщила, что своих ресурсов нет.
    pub fn hand_to_partner(
        &self,
        partner_id: PartnerId,
        at: Timestamp,
    ) -> Result<TicketEvent, TicketError> {
        self.ensure_open()?;
        if let Some(partner) = self.partner {
            return Err(TicketError::AlreadyWithPartner(partner));
        }
        if self.resources != Some(ResourceAvailability::Unavailable) {
            return Err(TicketError::PartnerNotNeeded(self.ticket_id));
        }
        Ok(TicketEvent::HandedToPartner(HandedToPartnerEvent {
            ticket_id: self.ticket_id,
            partner_id,
            at,
        }))
    }

    /// Решение об отчёте партнёра. Отчитаться может только партнёр, которому передан тикет.
    pub fn report_partner_progress(
        &self,
        partner_id: PartnerId,
        comment: &str,
        at: Timestamp,
    ) -> Result<TicketEvent, TicketError> {
        self.ensure_open()?;
        self.ensure_with_partner(partner_id)?;
        let comment = comment.trim();
        if comment.is_empty() {
            return Err(TicketError::EmptyComment);
        }
        Ok(TicketEvent::PartnerProgressReported(
            PartnerProgressReportedEvent {
                ticket_id: self.ticket_id,
                partner_id,
                comment: comment.to_owned(),
                at,
            },
        ))
    }

    /// Решение о закрытии тикета партнёром: тот же `TicketClosed`, что и при закрытии агентом,
    /// но закрыть может только партнёр, которому передан тикет.
    pub fn resolve_by_partner(
        &self,
        partner_id: PartnerId,
        at: Timestamp,
    ) -> Result<TicketEvent, TicketError> {
        self.ensure_can_become(TicketStatus::Closed)?;
        self.ensure_with_partner(partner_id)?;
        self.close(CloseReason::Resolved, at)
    }

    pub fn ticket_id(&self) -> TicketId {
        self.ticket_id
    }

    pub fn category(&self) -> &str {
        &self.category
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn status(&self) -> TicketStatus {
        self.status
    }

    pub fn assigned_agent(&self) -> Option<UserId> {
        self.assigned_agent
    }

    pub fn tenant(&self) -> TenantId {
        self.tenant
    }

    pub fn last_activity(&self) -> Timestamp {
        self.last_activity
    }

    pub fn tier(&self) -> CustomerTier {
        self.tier
    }

    pub fn sla(&self) -> Sla {
        self.sla
    }

    pub fn resources(&self) -> Option<ResourceAvailability> {
        self.resources
    }

    pub fn partner(&self) -> Option<PartnerId> {
        self.partner
    }

    fn ensure_with_partner(&self, partner_id: PartnerId) -> Result<(), TicketError> {
        if self.partner == Some(partner_id) {
            Ok(())
        } else {
            Err(TicketError::NotWithPartner(partner_id))
        }
    }

    /// Проверяет по правилам Shared Kernel, что тикет может перейти в статус `next`.
    ///
    /// Запрещённый переход из закрытого статуса означает, что тикет закрыт; из открытого —
    /// что его пытаются открыть ещё раз: других запрещённых переходов правила не оставляют.
    fn ensure_can_become(&self, next: TicketStatus) -> Result<(), TicketError> {
        self.status
            .transition_to(next)
            .map(drop)
            .map_err(|invalid| match invalid.from {
                TicketStatus::Closed => TicketError::Closed(self.ticket_id),
                TicketStatus::Open | TicketStatus::InProgress => {
                    TicketError::AlreadyOpen(self.ticket_id)
                }
            })
    }

    /// Напоминание статус не меняет, но закрытому тикету оно не нужно.
    fn ensure_open(&self) -> Result<(), TicketError> {
        if self.status.is_open() {
            Ok(())
        } else {
            Err(TicketError::Closed(self.ticket_id))
        }
    }
}

impl From<&TicketCreatedEvent> for Ticket {
    fn from(event: &TicketCreatedEvent) -> Self {
        Self {
            ticket_id: event.ticket_id,
            category: event.category.clone(),
            description: event.description.clone(),
            status: TicketStatus::Open,
            assigned_agent: None,
            tenant: event.tenant_id,
            tier: event.tier,
            sla: event.sla,
            last_activity: event.at,
            last_reminder: None,
            resources: None,
            partner: None,
        }
    }
}

/// Тикета может ещё не быть: агрегат — `Option<Ticket>`, где `None` соответствует пустому стриму.
impl Lifecycle for Ticket {
    type Event = TicketEvent;
    type Command = TicketCommand;
    type Error = TicketError;

    fn decide_new(command: &TicketCommand) -> Result<TicketEvent, TicketError> {
        match command {
            TicketCommand::Create {
                ticket_id,
                category,
                description,
                tenant_id,
                tier,
                at,
            } => Ticket::create(*ticket_id, category, description, *tenant_id, *tier, *at),
            _ => Err(TicketError::NotFound),
        }
    }

    fn decide(&self, command: &TicketCommand) -> Result<TicketEvent, TicketError> {
        match command {
            TicketCommand::Create { .. } => Err(TicketError::AlreadyExists(self.ticket_id)),
            TicketCommand::AssignAgent { agent, at } => self.assign_agent(agent, *at),
            TicketCommand::Close { reason, at } => self.close(*reason, *at),
            TicketCommand::Reopen { at } => self.reopen(*at),
            TicketCommand::SendReminder { kind, at } => self.send_reminder(*kind, *at),
            TicketCommand::RecordResourceCheck { availability, at } => {
                self.record_resource_check(*availability, *at)
            }
            TicketCommand::HandToPartner { partner_id, at } => {
                self.hand_to_partner(*partner_id, *at)
            }
            TicketCommand::ReportPartnerProgress {
                partner_id,
                comment,
                at,
            } => self.report_partner_progress(*partner_id, comment, *at),
            TicketCommand::ResolveByPartner { partner_id, at } => {
                self.resolve_by_partner(*partner_id, *at)
            }
        }
    }

    fn created(event: &TicketEvent) -> Self {
        match event {
            TicketEvent::Created(created) => Ticket::from(created),
            other => panic!("corrupted stream: {other} before creation"),
        }
    }

    fn evolve(self, event: &TicketEvent) -> Self {
        match event {
            TicketEvent::Created(_) => {
                panic!("corrupted stream: {} created twice", self.ticket_id)
            }
            TicketEvent::AgentAssigned(e) => Self {
                status: TicketStatus::InProgress,
                assigned_agent: Some(e.agent_id),
                last_activity: e.at,
                last_reminder: None,
                ..self
            },
            TicketEvent::Closed(_) => Self {
                status: TicketStatus::Closed,
                ..self
            },
            TicketEvent::Reopened(e) => Self {
                status: TicketStatus::Open,
                assigned_agent: None,
                sla: e.sla,
                last_activity: e.at,
                last_reminder: None,
                resources: None,
                partner: None,
                ..self
            },
            TicketEvent::ReminderSent(e) => Self {
                last_reminder: Some(e.kind),
                ..self
            },
            TicketEvent::ResourcesChecked(e) => Self {
                resources: Some(e.availability),
                ..self
            },
            TicketEvent::HandedToPartner(e) => Self {
                partner: Some(e.partner_id),
                ..self
            },
            TicketEvent::PartnerProgressReported(e) => Self {
                last_activity: e.at,
                last_reminder: None,
                ..self
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::user::{UserCommand, UserId};
    use framework::event_sourcing::{
        command_handler::CommandHandler, event_handler::EventHandler, event_sourced::EventSourced,
    };

    const T0: Timestamp = Timestamp(1_000);

    fn user(id: usize, role: Role) -> User {
        let command = UserCommand::Register {
            user_id: UserId(id),
            user_name: format!("u{id}"),
            role,
        };
        let event = None::<User>.decide(&command).unwrap();
        None::<User>.recover([event]).unwrap()
    }

    fn create_command() -> TicketCommand {
        TicketCommand::Create {
            ticket_id: TicketId(1),
            category: "Technical Issue".to_owned(),
            description: "Problem description".to_owned(),
            tenant_id: TenantId(1),
            tier: CustomerTier::Bronze,
            at: T0,
        }
    }

    /// Выполняет команду так же, как это делает репозиторий: decide, затем apply.
    fn execute(ticket: Option<Ticket>, command: TicketCommand) -> Option<Ticket> {
        let event = ticket.decide(&command).expect("command is accepted");
        ticket.apply_event(&event)
    }

    fn open_ticket() -> Option<Ticket> {
        execute(None, create_command())
    }

    fn closed_ticket() -> Option<Ticket> {
        execute(
            open_ticket(),
            TicketCommand::Close {
                reason: CloseReason::Resolved,
                at: T0,
            },
        )
    }

    fn assigned_ticket(agent: &User, at: Timestamp) -> Option<Ticket> {
        execute(
            open_ticket(),
            TicketCommand::AssignAgent {
                agent: agent.clone(),
                at,
            },
        )
    }

    #[test]
    fn create_on_empty_stream_emits_created_event() {
        assert_eq!(
            None::<Ticket>.decide(&create_command()),
            Ok(TicketEvent::Created(TicketCreatedEvent {
                ticket_id: TicketId(1),
                category: "Technical Issue".to_owned(),
                description: "Problem description".to_owned(),
                tenant_id: TenantId(1),
                tier: CustomerTier::Bronze,
                sla: Sla::for_tier(CustomerTier::Bronze, T0),
                at: T0,
            }))
        );
    }

    #[test]
    fn created_ticket_is_open_and_unassigned() {
        let ticket = open_ticket().unwrap();
        assert_eq!(ticket.ticket_id(), TicketId(1));
        assert_eq!(ticket.category(), "Technical Issue");
        assert_eq!(ticket.description(), "Problem description");
        assert_eq!(ticket.status(), TicketStatus::Open);
        assert_eq!(ticket.assigned_agent(), None);
        assert_eq!(ticket.tenant(), TenantId(1));
        assert_eq!(ticket.last_activity(), T0);
    }

    #[test]
    fn blank_description_is_rejected() {
        let command = TicketCommand::Create {
            ticket_id: TicketId(1),
            category: "Billing".to_owned(),
            description: "   ".to_owned(),
            tenant_id: TenantId(1),
            tier: CustomerTier::Bronze,
            at: T0,
        };
        assert_eq!(
            None::<Ticket>.decide(&command),
            Err(TicketError::EmptyDescription)
        );
    }

    #[test]
    fn blank_category_is_rejected() {
        let command = TicketCommand::Create {
            ticket_id: TicketId(1),
            category: "  ".to_owned(),
            description: "Problem description".to_owned(),
            tenant_id: TenantId(1),
            tier: CustomerTier::Bronze,
            at: T0,
        };
        assert_eq!(
            None::<Ticket>.decide(&command),
            Err(TicketError::EmptyCategory)
        );
    }

    #[test]
    fn ticket_cannot_be_created_twice() {
        assert_eq!(
            open_ticket().decide(&create_command()),
            Err(TicketError::AlreadyExists(TicketId(1)))
        );
    }

    #[test]
    fn commands_to_missing_ticket_are_rejected() {
        let assign = TicketCommand::AssignAgent {
            agent: user(2, Role::Agent),
            at: T0,
        };
        let close = TicketCommand::Close {
            reason: CloseReason::Resolved,
            at: T0,
        };
        let reopen = TicketCommand::Reopen { at: T0 };
        let remind = TicketCommand::SendReminder {
            kind: ReminderKind::Stale,
            at: T0,
        };
        assert_eq!(None::<Ticket>.decide(&remind), Err(TicketError::NotFound));
        assert_eq!(None::<Ticket>.decide(&assign), Err(TicketError::NotFound));
        assert_eq!(None::<Ticket>.decide(&close), Err(TicketError::NotFound));
        assert_eq!(None::<Ticket>.decide(&reopen), Err(TicketError::NotFound));
    }

    #[test]
    fn assign_agent_emits_event_and_moves_ticket_in_progress() {
        let agent = user(2, Role::Agent);
        let at = Timestamp(T0.0 + 60);
        let command = TicketCommand::AssignAgent {
            agent: agent.clone(),
            at,
        };
        assert_eq!(
            open_ticket().decide(&command),
            Ok(TicketEvent::AgentAssigned(AgentAssignedEvent {
                ticket_id: TicketId(1),
                agent_id: UserId(2),
                previous_agent_id: None,
                at,
            }))
        );

        let ticket = assigned_ticket(&agent, at).unwrap();
        assert_eq!(ticket.status(), TicketStatus::InProgress);
        assert_eq!(ticket.assigned_agent(), Some(UserId(2)));
        assert_eq!(ticket.last_activity(), at);
    }

    #[test]
    fn only_agents_can_be_assigned() {
        for role in [Role::Customer, Role::Admin] {
            let command = TicketCommand::AssignAgent {
                agent: user(3, role),
                at: T0,
            };
            assert_eq!(
                open_ticket().decide(&command),
                Err(TicketError::NotAnAgent {
                    user_id: UserId(3),
                    role
                })
            );
        }
    }

    #[test]
    fn same_agent_cannot_be_assigned_twice() {
        let agent = user(2, Role::Agent);
        let command = TicketCommand::AssignAgent {
            agent: agent.clone(),
            at: T0,
        };
        assert_eq!(
            assigned_ticket(&agent, T0).decide(&command),
            Err(TicketError::AlreadyAssigned(UserId(2)))
        );
    }

    #[test]
    fn reassignment_remembers_previous_agent() {
        let first = user(2, Role::Agent);
        let second = user(3, Role::Agent);
        let command = TicketCommand::AssignAgent {
            agent: second,
            at: T0,
        };
        let Ok(TicketEvent::AgentAssigned(event)) = assigned_ticket(&first, T0).decide(&command)
        else {
            panic!("reassignment is accepted");
        };
        assert_eq!(event.previous_agent_id, Some(UserId(2)));
        assert_eq!(event.agent_id, UserId(3));
    }

    #[test]
    fn close_moves_open_ticket_to_closed() {
        assert_eq!(open_ticket().unwrap().status(), TicketStatus::Open);
        assert_eq!(closed_ticket().unwrap().status(), TicketStatus::Closed);
    }

    #[test]
    fn closing_closed_ticket_is_rejected() {
        let close = TicketCommand::Close {
            reason: CloseReason::Resolved,
            at: T0,
        };
        assert_eq!(
            closed_ticket().decide(&close),
            Err(TicketError::Closed(TicketId(1)))
        );
    }

    #[test]
    fn reopen_moves_closed_ticket_to_open() {
        let at = Timestamp(T0.0 + 60);
        assert_eq!(
            closed_ticket().decide(&TicketCommand::Reopen { at }),
            Ok(TicketEvent::Reopened(TicketReopenedEvent {
                ticket_id: TicketId(1),
                tenant_id: TenantId(1),
                category: "Technical Issue".to_owned(),
                sla: Sla::for_tier(CustomerTier::Bronze, at),
                at,
            }))
        );
        let ticket = execute(closed_ticket(), TicketCommand::Reopen { at }).unwrap();
        assert_eq!(ticket.status(), TicketStatus::Open);
        assert_eq!(ticket.last_activity(), at);
    }

    #[test]
    fn reopened_ticket_starts_without_agent() {
        let close = TicketCommand::Close {
            reason: CloseReason::Resolved,
            at: T0,
        };
        let closed = execute(assigned_ticket(&user(2, Role::Agent), T0), close);
        let ticket = execute(closed, TicketCommand::Reopen { at: T0 }).unwrap();
        assert_eq!(ticket.status(), TicketStatus::Open);
        assert_eq!(ticket.assigned_agent(), None);
    }

    #[test]
    fn reopening_open_ticket_is_rejected() {
        let reopen = TicketCommand::Reopen { at: T0 };
        let in_progress = assigned_ticket(&user(2, Role::Agent), T0);
        for ticket in [open_ticket(), in_progress] {
            assert_eq!(
                ticket.decide(&reopen),
                Err(TicketError::AlreadyOpen(TicketId(1)))
            );
        }
    }

    #[test]
    fn reopened_ticket_can_be_closed_again() {
        let reopened = execute(closed_ticket(), TicketCommand::Reopen { at: T0 });
        let close = TicketCommand::Close {
            reason: CloseReason::Resolved,
            at: T0,
        };
        let ticket = execute(reopened, close).unwrap();
        assert_eq!(ticket.status(), TicketStatus::Closed);
    }

    #[test]
    fn resolved_ticket_closes_at_any_time() {
        let command = TicketCommand::Close {
            reason: CloseReason::Resolved,
            at: T0,
        };
        let ticket = execute(open_ticket(), command).unwrap();
        assert_eq!(ticket.status(), TicketStatus::Closed);
    }

    #[test]
    fn inactive_ticket_closes_after_timeout() {
        let at = Timestamp(T0.0 + INACTIVITY_TIMEOUT_SECS);
        let command = TicketCommand::Close {
            reason: CloseReason::Inactivity,
            at,
        };
        assert_eq!(
            open_ticket().decide(&command),
            Ok(TicketEvent::Closed(TicketClosedEvent {
                ticket_id: TicketId(1),
                tenant_id: TenantId(1),
                agent_id: None,
                reason: CloseReason::Inactivity,
                // 10 дней без активности — дольше, чем 5 дней SLA уровня Bronze.
                sla_met: false,
                at,
            }))
        );
    }

    #[test]
    fn active_ticket_cannot_be_closed_for_inactivity() {
        let at = Timestamp(T0.0 + INACTIVITY_TIMEOUT_SECS - 1);
        let command = TicketCommand::Close {
            reason: CloseReason::Inactivity,
            at,
        };
        assert_eq!(
            open_ticket().decide(&command),
            Err(TicketError::StillActive {
                idle_secs: INACTIVITY_TIMEOUT_SECS - 1
            })
        );
    }

    #[test]
    fn assignment_resets_inactivity_timer() {
        let assigned_at = Timestamp(T0.0 + INACTIVITY_TIMEOUT_SECS / 2);
        let ticket = assigned_ticket(&user(2, Role::Agent), assigned_at);
        let at = Timestamp(T0.0 + INACTIVITY_TIMEOUT_SECS);
        let command = TicketCommand::Close {
            reason: CloseReason::Inactivity,
            at,
        };
        assert!(matches!(
            ticket.decide(&command),
            Err(TicketError::StillActive { .. })
        ));
    }

    #[test]
    fn closed_ticket_rejects_further_commands() {
        let close = TicketCommand::Close {
            reason: CloseReason::Resolved,
            at: T0,
        };
        let ticket = execute(open_ticket(), close.clone());
        let assign = TicketCommand::AssignAgent {
            agent: user(2, Role::Agent),
            at: T0,
        };
        assert_eq!(
            ticket.decide(&assign),
            Err(TicketError::Closed(TicketId(1)))
        );
        assert_eq!(ticket.decide(&close), Err(TicketError::Closed(TicketId(1))));
    }

    #[test]
    fn recover_replays_whole_history() {
        let agent = user(2, Role::Agent);
        let events = [
            None::<Ticket>.decide(&create_command()).unwrap(),
            open_ticket()
                .decide(&TicketCommand::AssignAgent {
                    agent: agent.clone(),
                    at: T0,
                })
                .unwrap(),
            assigned_ticket(&agent, T0)
                .decide(&TicketCommand::Close {
                    reason: CloseReason::Resolved,
                    at: T0,
                })
                .unwrap(),
        ];
        let ticket = None::<Ticket>.recover(events).unwrap();
        assert_eq!(ticket.status(), TicketStatus::Closed);
        assert_eq!(ticket.assigned_agent(), Some(UserId(2)));
    }

    #[test]
    #[should_panic(expected = "corrupted stream")]
    fn event_before_creation_means_corrupted_stream() {
        let event = TicketEvent::Closed(TicketClosedEvent {
            ticket_id: TicketId(1),
            tenant_id: TenantId(1),
            agent_id: None,
            reason: CloseReason::Resolved,
            sla_met: true,
            at: T0,
        });
        let _ = None::<Ticket>.apply_event(&event);
    }

    /// Момент через `secs` секунд после создания тикета.
    fn after(secs: u64) -> Timestamp {
        Timestamp(T0.0 + secs)
    }

    fn remind(kind: ReminderKind, at: Timestamp) -> TicketCommand {
        TicketCommand::SendReminder { kind, at }
    }

    #[test]
    fn deadlines_follow_the_assignment() {
        assert_eq!(INACTIVITY_TIMEOUT_SECS, 10 * 24 * 60 * 60);
        assert_eq!(ReminderKind::Stale.due_after_secs(), 3 * 24 * 60 * 60);
        assert_eq!(ReminderKind::ClosingSoon.due_after_secs(), 9 * 24 * 60 * 60);
    }

    #[test]
    fn reminder_is_addressed_to_assigned_agent() {
        let at = after(STALE_REMINDER_AFTER_SECS);
        let ticket = assigned_ticket(&user(2, Role::Agent), T0);
        assert_eq!(
            ticket.decide(&remind(ReminderKind::Stale, at)),
            Ok(TicketEvent::ReminderSent(ReminderSentEvent {
                ticket_id: TicketId(1),
                agent_id: Some(UserId(2)),
                kind: ReminderKind::Stale,
                at,
            }))
        );
    }

    #[test]
    fn reminder_without_agent_is_addressed_to_admins() {
        let at = after(STALE_REMINDER_AFTER_SECS);
        assert_eq!(
            open_ticket().decide(&remind(ReminderKind::Stale, at)),
            Ok(TicketEvent::ReminderSent(ReminderSentEvent {
                ticket_id: TicketId(1),
                agent_id: None,
                kind: ReminderKind::Stale,
                at,
            }))
        );
    }

    #[test]
    fn reminder_before_its_time_is_rejected() {
        for kind in [ReminderKind::Stale, ReminderKind::ClosingSoon] {
            let idle_secs = kind.due_after_secs() - 1;
            assert_eq!(
                open_ticket().decide(&remind(kind, after(idle_secs))),
                Err(TicketError::ReminderNotDue { kind, idle_secs })
            );
            assert!(
                open_ticket()
                    .decide(&remind(kind, after(kind.due_after_secs())))
                    .is_ok()
            );
        }
    }

    #[test]
    fn closed_ticket_gets_no_reminder() {
        assert_eq!(
            closed_ticket().decide(&remind(ReminderKind::Stale, after(INACTIVITY_TIMEOUT_SECS))),
            Err(TicketError::Closed(TicketId(1)))
        );
    }

    #[test]
    fn reminder_is_not_repeated_and_goes_only_forward() {
        let at = after(INACTIVITY_TIMEOUT_SECS - 1);
        let stale = execute(open_ticket(), remind(ReminderKind::Stale, at));
        assert_eq!(
            stale.decide(&remind(ReminderKind::Stale, at)),
            Err(TicketError::ReminderAlreadySent(ReminderKind::Stale))
        );

        let closing = execute(stale, remind(ReminderKind::ClosingSoon, at));
        for kind in [ReminderKind::Stale, ReminderKind::ClosingSoon] {
            assert_eq!(
                closing.decide(&remind(kind, at)),
                Err(TicketError::ReminderAlreadySent(kind))
            );
        }
    }

    #[test]
    fn reminder_does_not_count_as_activity() {
        let reminded = execute(
            open_ticket(),
            remind(ReminderKind::Stale, after(STALE_REMINDER_AFTER_SECS)),
        );
        assert_eq!(reminded.as_ref().unwrap().last_activity(), T0);
        let close = TicketCommand::Close {
            reason: CloseReason::Inactivity,
            at: after(INACTIVITY_TIMEOUT_SECS),
        };
        assert!(reminded.decide(&close).is_ok());
    }

    #[test]
    fn assignment_starts_reminders_over() {
        let reminded = execute(
            open_ticket(),
            remind(ReminderKind::Stale, after(STALE_REMINDER_AFTER_SECS)),
        );
        let assigned_at = after(STALE_REMINDER_AFTER_SECS);
        let assigned = execute(
            reminded,
            TicketCommand::AssignAgent {
                agent: user(2, Role::Agent),
                at: assigned_at,
            },
        );

        let too_early = Timestamp(assigned_at.0 + STALE_REMINDER_AFTER_SECS - 1);
        assert_eq!(
            assigned.decide(&remind(ReminderKind::Stale, too_early)),
            Err(TicketError::ReminderNotDue {
                kind: ReminderKind::Stale,
                idle_secs: STALE_REMINDER_AFTER_SECS - 1,
            })
        );
        let due = Timestamp(assigned_at.0 + STALE_REMINDER_AFTER_SECS);
        assert!(assigned.decide(&remind(ReminderKind::Stale, due)).is_ok());
    }

    #[test]
    fn reopening_starts_reminders_over() {
        let at = after(STALE_REMINDER_AFTER_SECS);
        let reminded = execute(open_ticket(), remind(ReminderKind::Stale, at));
        let closed = execute(
            reminded,
            TicketCommand::Close {
                reason: CloseReason::Resolved,
                at,
            },
        );
        let reopened = execute(closed, TicketCommand::Reopen { at });
        let due = Timestamp(at.0 + STALE_REMINDER_AFTER_SECS);
        assert!(reopened.decide(&remind(ReminderKind::Stale, due)).is_ok());
    }

    #[test]
    fn closed_event_names_the_agent_who_led_the_ticket() {
        let close = TicketCommand::Close {
            reason: CloseReason::Resolved,
            at: T0,
        };
        let event = assigned_ticket(&user(2, Role::Agent), T0).decide(&close);
        assert!(matches!(
            event,
            Ok(TicketEvent::Closed(TicketClosedEvent {
                agent_id: Some(UserId(2)),
                ..
            }))
        ));
        let event = open_ticket().decide(&close);
        assert!(matches!(
            event,
            Ok(TicketEvent::Closed(TicketClosedEvent {
                agent_id: None,
                ..
            }))
        ));
    }

    #[test]
    fn reminder_event_is_readable_in_logs() {
        let event = |agent_id| {
            TicketEvent::ReminderSent(ReminderSentEvent {
                ticket_id: TicketId(1),
                agent_id,
                kind: ReminderKind::ClosingSoon,
                at: Timestamp(5),
            })
            .to_string()
        };
        assert_eq!(
            event(Some(UserId(2))),
            format!(
                "ReminderSent: ticket-1 (closing soon) to {} at t=5s",
                UserId(2)
            )
        );
        assert_eq!(
            event(None),
            "ReminderSent: ticket-1 (closing soon) to admins at t=5s"
        );
    }

    // --- SLA, ресурсы и партнёры ---

    const PARTNER: PartnerId = PartnerId(1);

    fn create_for(tier: CustomerTier) -> TicketCommand {
        TicketCommand::Create {
            ticket_id: TicketId(1),
            category: "Hardware".to_owned(),
            description: "Server is down".to_owned(),
            tenant_id: TenantId(1),
            tier,
            at: T0,
        }
    }

    fn close_at(at: Timestamp) -> TicketCommand {
        TicketCommand::Close {
            reason: CloseReason::Resolved,
            at,
        }
    }

    fn check(availability: ResourceAvailability) -> TicketCommand {
        TicketCommand::RecordResourceCheck {
            availability,
            at: T0,
        }
    }

    fn hand(partner_id: PartnerId) -> TicketCommand {
        TicketCommand::HandToPartner { partner_id, at: T0 }
    }

    fn report(partner_id: PartnerId, comment: &str, at: Timestamp) -> TicketCommand {
        TicketCommand::ReportPartnerProgress {
            partner_id,
            comment: comment.to_owned(),
            at,
        }
    }

    fn resolve(partner_id: PartnerId, at: Timestamp) -> TicketCommand {
        TicketCommand::ResolveByPartner { partner_id, at }
    }

    fn with_partner() -> Option<Ticket> {
        let checked = execute(open_ticket(), check(ResourceAvailability::Unavailable));
        execute(checked, hand(PARTNER))
    }

    #[test]
    fn sla_comes_from_customer_tier() {
        for tier in [
            CustomerTier::Gold,
            CustomerTier::Silver,
            CustomerTier::Bronze,
        ] {
            let ticket = execute(None, create_for(tier)).unwrap();
            assert_eq!(ticket.tier(), tier);
            assert_eq!(ticket.sla(), Sla::for_tier(tier, T0));
            assert_eq!(ticket.sla().due_at(), T0.plus_secs(tier.resolution_secs()));
        }
    }

    #[test]
    fn closing_reports_whether_sla_was_met() {
        let gold = execute(None, create_for(CustomerTier::Gold));
        let due = T0.plus_secs(CustomerTier::Gold.resolution_secs());
        let sla_met = |at| match gold.decide(&close_at(at)) {
            Ok(TicketEvent::Closed(e)) => e.sla_met,
            other => panic!("ticket is closed, got {other:?}"),
        };
        assert!(sla_met(due), "closing exactly at the deadline is in time");
        assert!(!sla_met(due.plus_secs(1)));
    }

    #[test]
    fn reopening_starts_new_sla_for_the_same_tier() {
        let gold = execute(None, create_for(CustomerTier::Gold));
        let closed = execute(gold, close_at(T0));
        let reopened_at = after(7 * DAY_SECS);
        let expected = Sla::for_tier(CustomerTier::Gold, reopened_at);
        assert!(matches!(
            closed.decide(&TicketCommand::Reopen { at: reopened_at }),
            Ok(TicketEvent::Reopened(TicketReopenedEvent { sla, .. })) if sla == expected
        ));
        let reopened = execute(closed, TicketCommand::Reopen { at: reopened_at }).unwrap();
        assert_eq!(reopened.sla(), expected);
        assert_eq!(reopened.tier(), CustomerTier::Gold);
    }

    #[test]
    fn resource_check_is_recorded_once() {
        assert_eq!(
            open_ticket().decide(&check(ResourceAvailability::Available)),
            Ok(TicketEvent::ResourcesChecked(ResourcesCheckedEvent {
                ticket_id: TicketId(1),
                availability: ResourceAvailability::Available,
                at: T0,
            }))
        );
        let checked = execute(open_ticket(), check(ResourceAvailability::Available));
        assert_eq!(
            checked.as_ref().unwrap().resources(),
            Some(ResourceAvailability::Available)
        );
        assert_eq!(
            checked.decide(&check(ResourceAvailability::Unavailable)),
            Err(TicketError::ResourcesAlreadyChecked(TicketId(1)))
        );
        assert_eq!(
            closed_ticket().decide(&check(ResourceAvailability::Available)),
            Err(TicketError::Closed(TicketId(1)))
        );
    }

    #[test]
    fn resource_check_is_not_activity() {
        let at = after(DAY_SECS);
        let checked = execute(
            open_ticket(),
            TicketCommand::RecordResourceCheck {
                availability: ResourceAvailability::Available,
                at,
            },
        );
        assert_eq!(checked.unwrap().last_activity(), T0);
    }

    #[test]
    fn partner_is_needed_only_when_resources_are_unavailable() {
        assert_eq!(
            open_ticket().decide(&hand(PARTNER)),
            Err(TicketError::PartnerNotNeeded(TicketId(1)))
        );
        let available = execute(open_ticket(), check(ResourceAvailability::Available));
        assert_eq!(
            available.decide(&hand(PARTNER)),
            Err(TicketError::PartnerNotNeeded(TicketId(1)))
        );
        let unavailable = execute(open_ticket(), check(ResourceAvailability::Unavailable));
        assert_eq!(
            unavailable.decide(&hand(PARTNER)),
            Ok(TicketEvent::HandedToPartner(HandedToPartnerEvent {
                ticket_id: TicketId(1),
                partner_id: PARTNER,
                at: T0,
            }))
        );
    }

    #[test]
    fn ticket_is_handed_to_one_partner_only() {
        let ticket = with_partner();
        assert_eq!(ticket.as_ref().unwrap().partner(), Some(PARTNER));
        assert_eq!(
            ticket.decide(&hand(PartnerId(2))),
            Err(TicketError::AlreadyWithPartner(PARTNER))
        );
    }

    #[test]
    fn partner_progress_is_activity_and_restarts_reminders() {
        let at = after(STALE_REMINDER_AFTER_SECS);
        let reminded = execute(with_partner(), remind(ReminderKind::Stale, at));
        assert_eq!(
            reminded.decide(&report(PARTNER, "  Replacement part ordered  ", at)),
            Ok(TicketEvent::PartnerProgressReported(
                PartnerProgressReportedEvent {
                    ticket_id: TicketId(1),
                    partner_id: PARTNER,
                    comment: "Replacement part ordered".to_owned(),
                    at,
                }
            ))
        );
        let reported = execute(reminded, report(PARTNER, "Replacement part ordered", at));
        assert_eq!(reported.as_ref().unwrap().last_activity(), at);
        let due_again = Timestamp(at.0 + STALE_REMINDER_AFTER_SECS);
        assert!(
            reported
                .decide(&remind(ReminderKind::Stale, due_again))
                .is_ok()
        );
        let close = TicketCommand::Close {
            reason: CloseReason::Inactivity,
            at: after(INACTIVITY_TIMEOUT_SECS),
        };
        assert!(matches!(
            reported.decide(&close),
            Err(TicketError::StillActive { .. })
        ));
    }

    #[test]
    fn only_the_assigned_partner_reports_and_with_some_text() {
        assert_eq!(
            open_ticket().decide(&report(PARTNER, "done", T0)),
            Err(TicketError::NotWithPartner(PARTNER))
        );
        assert_eq!(
            with_partner().decide(&report(PartnerId(2), "done", T0)),
            Err(TicketError::NotWithPartner(PartnerId(2)))
        );
        assert_eq!(
            with_partner().decide(&report(PARTNER, "   ", T0)),
            Err(TicketError::EmptyComment)
        );
    }

    #[test]
    fn partner_resolution_closes_ticket_as_resolved() {
        let at = after(DAY_SECS);
        let Ok(TicketEvent::Closed(event)) = with_partner().decide(&resolve(PARTNER, at)) else {
            panic!("partner resolves its ticket");
        };
        assert_eq!(event.reason, CloseReason::Resolved);
        assert_eq!(event.at, at);
        assert!(event.sla_met);
        let closed = execute(with_partner(), resolve(PARTNER, at)).unwrap();
        assert_eq!(closed.status(), TicketStatus::Closed);
    }

    #[test]
    fn partner_cannot_resolve_foreign_or_closed_ticket() {
        assert_eq!(
            open_ticket().decide(&resolve(PARTNER, T0)),
            Err(TicketError::NotWithPartner(PARTNER))
        );
        assert_eq!(
            with_partner().decide(&resolve(PartnerId(2), T0)),
            Err(TicketError::NotWithPartner(PartnerId(2)))
        );
        let closed = execute(with_partner(), resolve(PARTNER, T0));
        assert_eq!(
            closed.decide(&resolve(PARTNER, T0)),
            Err(TicketError::Closed(TicketId(1)))
        );
        assert_eq!(
            closed.decide(&report(PARTNER, "late note", T0)),
            Err(TicketError::Closed(TicketId(1)))
        );
    }

    #[test]
    fn reopened_ticket_forgets_resources_and_partner() {
        let closed = execute(with_partner(), resolve(PARTNER, T0));
        let reopened = execute(closed, TicketCommand::Reopen { at: T0 });
        let ticket = reopened.as_ref().unwrap();
        assert_eq!(ticket.resources(), None);
        assert_eq!(ticket.partner(), None);
        assert!(
            reopened
                .decide(&check(ResourceAvailability::Available))
                .is_ok()
        );
    }

    #[test]
    fn integration_events_are_readable_in_logs() {
        let checked = TicketEvent::ResourcesChecked(ResourcesCheckedEvent {
            ticket_id: TicketId(1),
            availability: ResourceAvailability::Unavailable,
            at: Timestamp(5),
        });
        assert_eq!(
            checked.to_string(),
            "ResourcesChecked: ticket-1 resources unavailable at t=5s"
        );
        let handed = TicketEvent::HandedToPartner(HandedToPartnerEvent {
            ticket_id: TicketId(1),
            partner_id: PARTNER,
            at: Timestamp(5),
        });
        assert_eq!(
            handed.to_string(),
            "HandedToPartner: ticket-1 -> partner-1 at t=5s"
        );
    }
}
