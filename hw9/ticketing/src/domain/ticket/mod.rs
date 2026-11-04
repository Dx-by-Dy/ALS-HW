mod command;
mod error;
mod events;
mod status;

use std::fmt;

pub use command::{CloseReason, ReminderKind, TicketCommand};
pub use error::TicketError;
pub use events::{
    AgentAssignedEvent, ReminderSentEvent, TicketClosedEvent, TicketCreatedEvent, TicketEvent,
    TicketReopenedEvent,
};
pub use status::TicketStatus;

use crate::domain::{
    tenant::TenantId,
    time::Timestamp,
    user::{Role, User, UserId},
};
use framework::event_sourcing::{event_store::StreamId, lifecycle::Lifecycle};

const DAY_SECS: u64 = 24 * 60 * 60;

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
    /// Время последнего изменения: от него отсчитывается неактивность.
    last_activity: Timestamp,
    /// Самое позднее напоминание, отправленное с момента последней активности.
    last_reminder: Option<ReminderKind>,
}

impl Ticket {
    /// Решение о создании тикета.
    pub fn create(
        ticket_id: TicketId,
        category: &str,
        description: &str,
        tenant_id: TenantId,
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
            at,
        }))
    }

    /// Решение о назначении агента. Повторное назначение другого агента разрешено.
    pub fn assign_agent(&self, agent: &User, at: Timestamp) -> Result<TicketEvent, TicketError> {
        self.ensure_open()?;
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
        self.ensure_open()?;
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
            at,
        }))
    }

    /// Решение об открытии закрытого тикета. Он начинает жизнь заново: без агента
    /// и с новым отсчётом неактивности.
    pub fn reopen(&self, at: Timestamp) -> Result<TicketEvent, TicketError> {
        if self.status.is_open() {
            return Err(TicketError::AlreadyOpen(self.ticket_id));
        }
        Ok(TicketEvent::Reopened(TicketReopenedEvent {
            ticket_id: self.ticket_id,
            tenant_id: self.tenant,
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
            last_activity: event.at,
            last_reminder: None,
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
                at,
            } => Ticket::create(*ticket_id, category, description, *tenant_id, *at),
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
                last_activity: e.at,
                last_reminder: None,
                ..self
            },
            TicketEvent::ReminderSent(e) => Self {
                last_reminder: Some(e.kind),
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
}
