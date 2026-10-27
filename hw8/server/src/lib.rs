//! HTTP-адаптер над CQRS: команды уходят в актор командной стороны, запросы читают read-модель.
//!
//! [`app`] собирает систему целиком. Кроме актора, в отдельных задачах tokio работают
//! проекция (события → read-модель), политика скидок (события → команды) и расписание,
//! которое закрывает неактивные тикеты. Клиенты узнают об изменениях через long polling
//! `GET /api/changes?since=N`.

mod commands;
mod dto;
mod error;
mod queries;
mod seed;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use command::{Command, CommandActorHandle, CommandError};
use framework::cqrs::{
    envelope::Version,
    runner::{run_policy, run_projection},
};
use query::ReadModel;
use ticketing::application::{
    clock::Clock, discount_policy::DiscountPolicy, error::ServiceError,
    ticket_service::TicketService,
};
use ticketing::domain::{DomainEvent, tenant::TenantError};

pub use error::ApiError;
pub use seed::seed;

type ApiResult<T> = Result<Json<T>, ApiError>;
type Shared = State<Arc<AppState>>;

#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// Сколько long polling держит запрос, если изменений нет.
    pub long_poll: Duration,
    /// Как часто проверять тикеты на неактивность.
    pub close_inactive_every: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            long_poll: Duration::from_secs(25),
            close_inactive_every: Duration::from_secs(60),
        }
    }
}

/// Запущенная система: HTTP-роутер и стороны CQRS, к которым он обращается.
pub struct App {
    pub router: Router,
    pub commands: CommandActorHandle,
    pub read_model: ReadModel,
}

struct AppState {
    commands: CommandActorHandle,
    read_model: ReadModel,
    /// Для запросов «за последние N дней»: read-модель о текущем времени не знает.
    clock: Arc<dyn Clock + Send + Sync>,
    long_poll: Duration,
    /// Идентификатор запуска: версии разных запусков несравнимы (см. `api::Cursor`).
    epoch: u64,
}

/// Запускает командную сторону, проекцию, политику скидок и расписание в задачах tokio
/// и возвращает роутер над ними. Должна вызываться внутри рантайма tokio.
pub fn app<C: Clock + Clone + Send + Sync + 'static>(clock: C, config: Config) -> App {
    let commands = command::spawn(TicketService::new(clock.clone()));
    let read_model = ReadModel::default();
    spawn_projection(&commands, &read_model);
    spawn_discount_policy(&commands);
    tokio::spawn(command::repeat(
        commands.clone(),
        config.close_inactive_every,
        Command::CloseInactiveTickets,
    ));

    let state = Arc::new(AppState {
        commands: commands.clone(),
        read_model: read_model.clone(),
        clock: Arc::new(clock),
        long_poll: config.long_poll,
        epoch: new_epoch(),
    });
    let router = Router::new()
        .route("/api/tickets/{id}/open", post(commands::open_ticket))
        .route("/api/tickets/{id}/close", post(commands::close_ticket))
        .route("/api/tickets/{id}/assign", post(commands::assign_agent))
        .route("/api/tickets", get(queries::list_tickets))
        .route("/api/tickets/closed", get(queries::tickets_closed_within))
        .route("/api/tickets/{id}", get(queries::get_ticket))
        .route(
            "/api/agents/{id}/tickets",
            get(queries::active_tickets_of_agent),
        )
        .route("/api/tenants", get(queries::list_tenants))
        .route("/api/tenants/{id}", get(queries::get_tenant))
        .route(
            "/api/tenants/{id}/tickets/unassigned",
            get(queries::unassigned_tickets_of_tenant),
        )
        .route("/api/users", get(queries::list_users))
        .route("/api/changes", get(queries::changes))
        .with_state(state);
    App {
        router,
        commands,
        read_model,
    }
}

/// Момент запуска в наносекундах: у каждого запуска сервера своя эпоха.
fn new_epoch() -> u64 {
    let since_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is after the Unix epoch");
    since_unix.as_nanos() as u64
}

fn spawn_projection(commands: &CommandActorHandle, read_model: &ReadModel) {
    let (source, model) = (commands.clone(), read_model.clone());
    tokio::spawn(async move {
        if let Err(e) = run_projection::<DomainEvent, _, _>(&source, &model).await {
            eprintln!("error: read model stopped: {e}");
        }
    });
}

fn spawn_discount_policy(commands: &CommandActorHandle) {
    let bus = commands.clone();
    tokio::spawn(async move {
        let result =
            run_policy::<DomainEvent, _, _, _>(&bus, Version(0), &DiscountPolicy, &bus, |e| {
                if !is_discount_unchanged(&e) {
                    eprintln!("warning: discount policy command failed: {e}");
                }
            })
            .await;
        if let Err(e) = result {
            eprintln!("error: discount policy stopped: {e}");
        }
    });
}

/// Скидка осталась прежней — обычный исход пересчёта, а не сбой.
fn is_discount_unchanged(e: &CommandError) -> bool {
    matches!(
        e,
        CommandError::Rejected(ServiceError::Tenant(TenantError::DiscountUnchanged(_)))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{
        ChangesDto, CommandAcceptedDto, Cursor, ErrorDto, TenantDto, TicketDto, TicketStatus,
        UserDto,
    };
    use axum::{
        body::Body,
        http::{Request, StatusCode, header},
    };
    use http_body_util::BodyExt;
    use serde::de::DeserializeOwned;
    use ticketing::application::clock::ManualClock;
    use tower::ServiceExt;

    /// Номера тикетов и пользователей из [`seed`].
    const ASSIGNED: usize = 1;
    const OPEN: usize = 2;
    const CLOSED: usize = 7;
    const AGENT_ALICE: usize = 1;
    const AGENT_CAROL: usize = 2;
    const CUSTOMER_BOB: usize = 3;
    const ACME: usize = 1;
    const GLOBEX: usize = 2;
    const DAY: u64 = 24 * 60 * 60;
    const WAIT: Duration = Duration::from_secs(5);

    /// Система с демо-данными, в которой всё уже разошлось: read-модель догнала seed,
    /// политика назначила скидку Acme.
    struct TestApp {
        router: Router,
        clock: ManualClock,
        /// Позиция после того, как всё разошлось.
        settled: Cursor,
    }

    async fn settled_app(long_poll: Duration) -> TestApp {
        let clock = ManualClock::default();
        let config = Config {
            long_poll,
            close_inactive_every: Duration::from_secs(3600),
        };
        let app = app(clock.clone(), config);
        seed(&app.commands).await.unwrap();
        let router = app.router;
        wait_for_tenant(&router, ACME, |t| t.discount_percent == 10).await;
        let all: ChangesDto = get(&router, &api::changes(None)).await;
        TestApp {
            router,
            clock,
            settled: all.cursor(),
        }
    }

    async fn seeded_app() -> TestApp {
        settled_app(WAIT).await
    }

    async fn send<T: DeserializeOwned>(
        app: &Router,
        method: &str,
        uri: &str,
        body: Option<&str>,
    ) -> (StatusCode, T) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_owned())))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    async fn get<T: DeserializeOwned>(app: &Router, uri: &str) -> T {
        let (status, body) = send(app, "GET", uri, None).await;
        assert_eq!(status, StatusCode::OK, "GET {uri}");
        body
    }

    impl TestApp {
        async fn get<T: DeserializeOwned>(&self, uri: &str) -> T {
            get(&self.router, uri).await
        }

        async fn send<T: DeserializeOwned>(
            &self,
            method: &str,
            uri: &str,
            body: Option<&str>,
        ) -> (StatusCode, T) {
            send(&self.router, method, uri, body).await
        }

        /// Выполняет команду и ждёт, пока её результат дойдёт до read-модели.
        async fn command(&self, uri: &str, body: Option<&str>) -> u64 {
            let (status, accepted): (_, CommandAcceptedDto) = self.send("POST", uri, body).await;
            assert_eq!(status, StatusCode::OK, "POST {uri}");
            // Long polling с версии перед командой ответит, как только модель её догонит.
            let before = Cursor {
                version: accepted.version - 1,
                ..self.settled
            };
            let changes: ChangesDto = self.get(&api::changes(Some(before))).await;
            assert!(changes.version >= accepted.version);
            accepted.version
        }
    }

    /// Следит за изменениями через long polling, пока тенант не придёт в нужное состояние.
    async fn wait_for_tenant(
        app: &Router,
        tenant_id: usize,
        check: impl Fn(&TenantDto) -> bool,
    ) -> TenantDto {
        let wait = async {
            let mut after = None;
            loop {
                let changes: ChangesDto = get(app, &api::changes(after)).await;
                let found = changes.tenants.iter().find(|t| t.tenant_id == tenant_id);
                if let Some(tenant) = found.filter(|t| check(t)) {
                    return tenant.clone();
                }
                after = Some(changes.cursor());
            }
        };
        tokio::time::timeout(WAIT, wait)
            .await
            .expect("tenant reached expected state")
    }

    fn ids(tickets: &[TicketDto]) -> Vec<usize> {
        tickets.iter().map(|t| t.ticket_id).collect()
    }

    #[tokio::test]
    async fn seeded_state_is_served_from_read_model() {
        let app = seeded_app().await;
        let tickets: Vec<TicketDto> = app.get(api::TICKETS).await;
        assert_eq!(tickets.len(), 9);
        assert_eq!(tickets[ASSIGNED - 1].status, TicketStatus::InProgress);
        assert_eq!(tickets[ASSIGNED - 1].assigned_agent, Some(AGENT_ALICE));
        assert_eq!(tickets[CLOSED - 1].status, TicketStatus::Closed);
        assert_eq!(tickets[CLOSED - 1].closed_at, Some(0));

        let users: Vec<UserDto> = app.get(api::USERS).await;
        assert_eq!(users.len(), 4);

        let tenants: Vec<TenantDto> = app.get(api::TENANTS).await;
        assert_eq!(tenants[ACME - 1].open_tickets, 6);
        assert_eq!(tenants[ACME - 1].discount_percent, 10);
        assert_eq!(tenants[GLOBEX - 1].discount_percent, 0);
    }

    #[tokio::test]
    async fn command_answers_version_and_result_is_readable_after_it() {
        let app = seeded_app().await;
        app.command(&api::close_ticket(OPEN), None).await;
        let ticket: TicketDto = app.get(&api::ticket(OPEN)).await;
        assert_eq!(ticket.status, TicketStatus::Closed);

        let (status, error): (_, ErrorDto) = app.send("POST", &api::close_ticket(OPEN), None).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error.error, "ticket-2 is closed");

        app.command(&api::open_ticket(OPEN), None).await;
        let ticket: TicketDto = app.get(&api::ticket(OPEN)).await;
        assert_eq!(ticket.status, TicketStatus::Open);
        assert_eq!(ticket.closed_at, None);
    }

    #[tokio::test]
    async fn opening_open_ticket_conflicts() {
        let app = seeded_app().await;
        let (status, error): (_, ErrorDto) = app.send("POST", &api::open_ticket(OPEN), None).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error.error, "ticket-2 is already open");
    }

    #[tokio::test]
    async fn assign_accepts_only_agents() {
        let app = seeded_app().await;
        let body = format!(r#"{{"agent_id":{AGENT_CAROL}}}"#);
        app.command(&api::assign_agent(OPEN), Some(&body)).await;
        let ticket: TicketDto = app.get(&api::ticket(OPEN)).await;
        assert_eq!(ticket.assigned_agent, Some(AGENT_CAROL));
        assert_eq!(ticket.status, TicketStatus::InProgress);

        let body = format!(r#"{{"agent_id":{CUSTOMER_BOB}}}"#);
        let (status, _): (_, ErrorDto) = app
            .send("POST", &api::assign_agent(OPEN), Some(&body))
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn unknown_ticket_and_tenant_are_not_found() {
        let app = seeded_app().await;
        let (status, error): (_, ErrorDto) = app.send("POST", &api::close_ticket(99), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(error.error, "ticket does not exist");

        let (status, error): (_, ErrorDto) = app.send("GET", &api::ticket(99), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(error.error, "ticket-99 does not exist");
        let (status, _): (_, ErrorDto) = app.send("GET", &api::tenant(99), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn read_side_queries() {
        let app = seeded_app().await;
        let active: Vec<TicketDto> = app.get(&api::active_tickets_of_agent(AGENT_ALICE)).await;
        assert_eq!(ids(&active), [ASSIGNED]);
        let unassigned: Vec<TicketDto> = app.get(&api::unassigned_tickets_of_tenant(ACME)).await;
        assert_eq!(ids(&unassigned), [2, 3, 4, 5, 6]);

        let closed: Vec<TicketDto> = app.get("/api/tickets?status=Closed").await;
        assert_eq!(ids(&closed), [CLOSED]);
        let globex: Vec<TicketDto> = app.get(&format!("/api/tickets?tenant={GLOBEX}")).await;
        assert_eq!(ids(&globex), [8, 9]);
        let in_progress: Vec<TicketDto> = app
            .get(&format!("/api/tickets?status=InProgress&tenant={ACME}"))
            .await;
        assert_eq!(ids(&in_progress), [ASSIGNED]);

        let acme: TenantDto = app.get(&api::tenant(ACME)).await;
        assert_eq!(acme.tenant_name, "Acme");
    }

    #[tokio::test]
    async fn closed_within_days_is_counted_from_now() {
        let app = seeded_app().await;
        app.clock.advance(8 * DAY);
        app.command(&api::close_ticket(OPEN), None).await;

        let last_week: Vec<TicketDto> = app.get(&api::tickets_closed_within(7)).await;
        assert_eq!(ids(&last_week), [OPEN]);
        let last_month: Vec<TicketDto> = app.get(&api::tickets_closed_within(30)).await;
        assert_eq!(ids(&last_month), [CLOSED, OPEN]);
    }

    #[tokio::test]
    async fn first_request_gets_everything_at_once() {
        let app = seeded_app().await;
        let all: ChangesDto = app.get(&api::changes(None)).await;
        assert!(all.full);
        assert_eq!(all.cursor(), app.settled);
        assert_eq!(
            (all.tickets.len(), all.tenants.len(), all.users.len()),
            (9, 2, 4)
        );
    }

    #[tokio::test]
    async fn long_poll_answers_as_soon_as_something_changes() {
        let app = seeded_app().await;
        let poll = tokio::spawn({
            let (router, uri) = (app.router.clone(), api::changes(Some(app.settled)));
            async move { get::<ChangesDto>(&router, &uri).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!poll.is_finished(), "nothing changed yet, request is held");

        let (_, accepted): (_, CommandAcceptedDto) =
            app.send("POST", &api::close_ticket(OPEN), None).await;
        let changes = tokio::time::timeout(WAIT, poll).await.unwrap().unwrap();
        assert!(!changes.full);
        assert!(changes.version >= accepted.version);
        let ticket = changes
            .tickets
            .iter()
            .find(|t| t.ticket_id == OPEN)
            .unwrap();
        assert_eq!(ticket.status, TicketStatus::Closed);
    }

    #[tokio::test]
    async fn long_poll_times_out_with_empty_changes() {
        let app = settled_app(Duration::from_millis(100)).await;
        let started = tokio::time::Instant::now();
        let changes: ChangesDto = app.get(&api::changes(Some(app.settled))).await;
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert_eq!(changes.cursor(), app.settled);
        assert!(!changes.full);
        assert!(changes.tickets.is_empty() && changes.tenants.is_empty());
    }

    #[tokio::test]
    async fn cursor_from_another_server_run_gets_everything() {
        let app = seeded_app().await;
        // Версия меньше текущей, но из другого запуска: дельта от неё была бы чужой историей.
        let stale = Cursor {
            epoch: app.settled.epoch + 1,
            version: 3,
        };
        let changes: ChangesDto = app.get(&api::changes(Some(stale))).await;
        assert!(changes.full);
        assert_eq!(changes.cursor(), app.settled);
        assert_eq!(changes.tickets.len(), 9);
    }

    #[tokio::test]
    async fn discount_is_recalculated_by_policy() {
        let app = seeded_app().await;
        app.command(&api::close_ticket(OPEN), None).await;
        let acme = wait_for_tenant(&app.router, ACME, |t| t.discount_percent == 0).await;
        assert_eq!(acme.open_tickets, 5);

        app.command(&api::open_ticket(OPEN), None).await;
        let acme = wait_for_tenant(&app.router, ACME, |t| t.discount_percent == 10).await;
        assert_eq!(acme.open_tickets, 6);
    }
}
