use std::convert::Infallible;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use framework::cqrs::{
    envelope::{Envelope, Version},
    projection::Projection,
    query::{Query, QueryHandler},
};
use ticketing::domain::DomainEvent;
use tokio::sync::watch;

use crate::queries::{
    ActiveTicketsOfAgent, ChangesSince, GetTenant, GetTicket, ListTenants, ListTickets, ListUsers,
    TicketsClosedSince, UnassignedTicketsOfTenant,
};
use crate::state::{ProjectionError, State};

/// Read-модель в памяти процесса. Клоны делят одно состояние: один клон обновляет
/// проектор, остальные читают обработчики запросов.
///
/// Блокировка берётся только на время синхронного обновления или чтения и никогда
/// не держится через `await`, поэтому писатель и читатели почти не мешают друг другу.
#[derive(Debug, Clone, Default)]
pub struct ReadModel {
    inner: Arc<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    state: RwLock<State>,
    /// Версия модели для тех, кто ждёт изменений.
    version: watch::Sender<Version>,
}

impl ReadModel {
    /// Ждёт, пока модель дойдёт до версии `version`, и возвращает её текущую версию.
    ///
    /// На этом строятся и read-your-writes (дождаться версии своей команды), и long polling
    /// (дождаться версии после последней увиденной). Ограничить ожидание по времени —
    /// забота вызывающего.
    pub async fn wait_until(&self, version: Version) -> Version {
        let mut current = self.inner.version.subscribe();
        let reached = current
            .wait_for(|current| *current >= version)
            .await
            .expect("version sender lives as long as the read model");
        *reached
    }

    fn read(&self) -> RwLockReadGuard<'_, State> {
        self.inner
            .state
            .read()
            .expect("read model lock is not poisoned")
    }

    fn write(&self) -> RwLockWriteGuard<'_, State> {
        self.inner
            .state
            .write()
            .expect("read model lock is not poisoned")
    }
}

impl Projection<DomainEvent> for ReadModel {
    type Error = ProjectionError;

    async fn project(&self, event: &Envelope<DomainEvent>) -> Result<(), ProjectionError> {
        let version = {
            let mut state = self.write();
            state.apply(event)?;
            state.version()
        };
        // Ждущих будим, только когда изменения уже видны читателям.
        self.inner.version.send_replace(version);
        Ok(())
    }

    async fn version(&self) -> Version {
        self.read().version()
    }
}

/// Все запросы отвечают по снимку состояния под одной блокировкой чтения
/// и не могут завершиться ошибкой: данные в памяти.
macro_rules! impl_query_handler {
    ($($query:ty => |$state:ident, $q:ident| $answer:expr;)*) => {
        $(
            impl QueryHandler<$query> for ReadModel {
                type Error = Infallible;

                async fn handle(
                    &self,
                    $q: $query,
                ) -> Result<<$query as Query>::Output, Infallible> {
                    let $state = self.read();
                    Ok($answer)
                }
            }
        )*
    };
}

impl_query_handler! {
    GetTicket => |state, query| state.ticket(query.0);
    ListTickets => |state, query| state.tickets(query);
    ActiveTicketsOfAgent => |state, query| state.active_tickets_of_agent(query.0);
    UnassignedTicketsOfTenant => |state, query| state.unassigned_tickets_of_tenant(query.0);
    TicketsClosedSince => |state, query| state.tickets_closed_since(query.0);
    GetTenant => |state, query| state.tenant(query.0);
    ListTenants => |state, _query| state.tenants();
    ListUsers => |state, _query| state.users();
    ChangesSince => |state, query| state.changes_since(query.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::tests::{ACME, ALICE, agent, assigned, closed, created, tenant};
    use ticketing::domain::{ticket::TicketId, time::Timestamp};

    async fn model_with(events: Vec<DomainEvent>) -> ReadModel {
        let model = ReadModel::default();
        for (version, event) in (1..).map(Version).zip(events) {
            model.project(&Envelope { version, event }).await.unwrap();
        }
        model
    }

    async fn ask<Q: Query>(model: &ReadModel, query: Q) -> Q::Output
    where
        ReadModel: QueryHandler<Q, Error = Infallible>,
    {
        let Ok(answer) = model.handle(query).await;
        answer
    }

    #[tokio::test]
    async fn queries_are_answered_from_projected_events() {
        let model = model_with(vec![
            tenant(ACME, "Acme"),
            agent(ALICE, "Alice"),
            created(1, ACME, 0),
            created(2, ACME, 0),
            assigned(1, ALICE, None),
            closed(2, ACME, 100),
        ])
        .await;
        assert_eq!(model.version().await, Version(6));

        let active = ask(&model, ActiveTicketsOfAgent(ALICE)).await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].ticket_id, TicketId(1));
        let closed = ask(&model, TicketsClosedSince(Timestamp(100))).await;
        assert_eq!(closed[0].ticket_id, TicketId(2));
        assert!(
            ask(&model, UnassignedTicketsOfTenant(ACME))
                .await
                .is_empty()
        );
        assert_eq!(ask(&model, GetTenant(ACME)).await.unwrap().open_tickets, 1);
        assert_eq!(ask(&model, ListTickets::default()).await.len(), 2);
        assert_eq!(ask(&model, ListUsers).await.len(), 1);
        assert_eq!(ask(&model, ChangesSince(Version(5))).await.tickets.len(), 1);
    }

    #[tokio::test]
    async fn failed_event_keeps_version() {
        let model = model_with(vec![tenant(ACME, "Acme")]).await;
        let orphan = Envelope {
            version: Version(2),
            event: closed(9, ACME, 0),
        };
        assert_eq!(
            model.project(&orphan).await,
            Err(ProjectionError::UnknownTicket(TicketId(9)))
        );
        assert_eq!(model.version().await, Version(1));
    }

    #[tokio::test]
    async fn wait_until_returns_at_once_for_reached_version() {
        let model = model_with(vec![tenant(ACME, "Acme"), agent(ALICE, "Alice")]).await;
        assert_eq!(model.wait_until(Version(1)).await, Version(2));
    }

    #[tokio::test]
    async fn wait_until_wakes_up_when_version_is_projected() {
        let model = model_with(vec![tenant(ACME, "Acme")]).await;
        let waiter = tokio::spawn({
            let model = model.clone();
            async move { model.wait_until(Version(2)).await }
        });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        let event = Envelope {
            version: Version(2),
            event: created(1, ACME, 0),
        };
        model.project(&event).await.unwrap();
        assert_eq!(waiter.await.unwrap(), Version(2));
        let ticket = ask(&model, GetTicket(TicketId(1))).await;
        assert!(ticket.is_some(), "change is visible once waiter wakes up");
    }
}
