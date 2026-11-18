//! HTTP-адаптер над CQRS: команды уходят в актор командной стороны, запросы читают read-модель.
//!
//! [`app`] собирает систему целиком. Кроме актора, в отдельных задачах tokio работают
//! проекция (события → read-модель), политика скидок (события → команды), уведомления
//! (события → журнал сервера), оркестратор жизненного цикла тикетов с напоминаниями
//! и автозакрытием, планирование ресурсов (события → ERP → команды) и сообщения клиентам
//! (события → CRM). Клиенты узнают об изменениях через long polling
//! `GET /api/changes?since=N`.

mod commands;
mod dto;
mod error;
mod notifier;
mod partner;
mod queries;
mod seed;
mod world;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use command::{CommandActorHandle, CommandError};
use framework::cqrs::{
    envelope::Version,
    runner::{run_policy, run_projection},
};
use query::ReadModel;
use ticketing::application::{
    clock::Clock, customer_notices::run_customer_notices, discount_policy::DiscountPolicy,
    error::ServiceError, notifications::run_notifications, resource_planning::PlanningError,
    resource_planning::run_resource_planning, ticket_service::TicketService,
};
use ticketing::domain::{DomainEvent, tenant::TenantError, ticket::TicketError};

pub use error::ApiError;
use notifier::LogNotifier;
pub use seed::seed;
pub use world::{ExternalSystems, FIXIT, FIXIT_TOKEN, NETCO, NETCO_TOKEN};

type ApiResult<T> = Result<Json<T>, ApiError>;
type Shared = State<Arc<AppState>>;

#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// Сколько long polling держит запрос, если изменений нет.
    pub long_poll: Duration,
    /// Как часто проверять, не наступили ли дедлайны тикетов.
    pub deadline_tick: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            long_poll: Duration::from_secs(25),
            deadline_tick: Duration::from_secs(1),
        }
    }
}

impl Config {
    /// Настройки для часов, идущих в `scale` раз быстрее настоящих. Дедлайны срабатывают на тике,
    /// поэтому он сокращается вместе с масштабом: опоздание не больше минуты времени системы.
    /// Чаще раза в 10 мс проверять незачем: ниже этого масштаб всё равно нельзя заметить.
    pub fn for_time_scale(scale: u32) -> Self {
        const MAX_LAG: Duration = Duration::from_secs(60);
        const MIN_TICK: Duration = Duration::from_millis(10);
        let default = Self::default();
        Self {
            deadline_tick: (MAX_LAG / scale.max(1)).clamp(MIN_TICK, default.deadline_tick),
            ..default
        }
    }
}

/// Запущенная система: HTTP-роутер и стороны CQRS, к которым он обращается.
pub struct App {
    pub router: Router,
    pub commands: CommandActorHandle,
    pub read_model: ReadModel,
    /// CRM, ERP и партнёры, с которыми работает система.
    pub external: ExternalSystems,
}

struct AppState {
    commands: CommandActorHandle,
    read_model: ReadModel,
    /// Справочник клиентов для приёма тикетов.
    crm: world::Crm,
    external: ExternalSystems,
    /// Для запросов «за последние N дней»: read-модель о текущем времени не знает.
    clock: Arc<dyn Clock + Send + Sync>,
    long_poll: Duration,
    /// Идентификатор запуска: версии разных запусков несравнимы (см. `api::Cursor`).
    epoch: u64,
}

/// Запускает командную сторону, проекцию, политики, уведомления, оркестратор и интеграции
/// в задачах tokio и возвращает роутер над ними. Внешний мир — [`ExternalSystems::demo`].
/// Должна вызываться внутри рантайма tokio.
pub fn app<C: Clock + Clone + Send + Sync + 'static>(clock: C, config: Config) -> App {
    let commands = command::spawn(TicketService::new(clock.clone()));
    let read_model = ReadModel::default();
    let external = ExternalSystems::demo();
    spawn_projection(&commands, &read_model);
    spawn_discount_policy(&commands);
    spawn_notifications(&commands);
    spawn_ticket_lifecycle(&commands, clock.clone(), config.deadline_tick);
    spawn_resource_planning(&commands, &external);
    spawn_customer_notices(&commands, &external);

    let state = Arc::new(AppState {
        commands: commands.clone(),
        read_model: read_model.clone(),
        crm: external.crm_adapter(),
        external: external.clone(),
        clock: Arc::new(clock),
        long_poll: config.long_poll,
        epoch: new_epoch(),
    });
    let router = Router::new()
        .route("/api/tickets/{id}/open", post(commands::open_ticket))
        .route("/api/tickets/{id}/close", post(commands::close_ticket))
        .route("/api/tickets/{id}/assign", post(commands::assign_agent))
        .route(
            "/api/tickets",
            get(queries::list_tickets).post(commands::create_ticket),
        )
        .route("/api/tickets/closed", get(queries::tickets_closed_within))
        .route("/api/tickets/{id}", get(queries::get_ticket))
        .route(
            "/api/agents/{id}/tickets",
            get(queries::active_tickets_of_agent),
        )
        .route(
            "/api/tenants",
            get(queries::list_tenants).post(commands::create_tenant),
        )
        .route("/api/tenants/{id}", get(queries::get_tenant))
        .route(
            "/api/tenants/{id}/tickets/unassigned",
            get(queries::unassigned_tickets_of_tenant),
        )
        .route(
            "/api/users",
            get(queries::list_users).post(commands::create_user),
        )
        .route(
            "/api/users/{id}/notifications",
            get(queries::notifications_of),
        )
        .route("/api/changes", get(queries::changes))
        .route(partner_api::TICKETS, get(partner::list_tickets))
        .route("/partner/v1/tickets/{id}", get(partner::get_ticket))
        .route(
            "/partner/v1/tickets/{id}/updates",
            post(partner::post_update),
        )
        .with_state(state);
    App {
        router,
        commands,
        read_model,
        external,
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

fn spawn_notifications(commands: &CommandActorHandle) {
    let source = commands.clone();
    tokio::spawn(async move {
        let result = run_notifications(&source, Version(0), &LogNotifier, |_, e| match e {}).await;
        if let Err(e) = result {
            eprintln!("error: notifications stopped: {e}");
        }
    });
}

fn spawn_ticket_lifecycle<C>(commands: &CommandActorHandle, clock: C, tick: Duration)
where
    C: Clock + Send + 'static,
{
    let bus = commands.clone();
    tokio::spawn(async move {
        let result = command::run_ticket_lifecycle(bus, clock, tick, |e| {
            if !is_outdated_deadline(&e) {
                eprintln!("warning: ticket lifecycle command failed: {e}");
            }
        })
        .await;
        if let Err(e) = result {
            eprintln!("error: ticket lifecycle stopped: {e}");
        }
    });
}

fn spawn_resource_planning(commands: &CommandActorHandle, external: &ExternalSystems) {
    let bus = commands.clone();
    let (erp, partners) = (external.erp_adapter(), external.partners.clone());
    tokio::spawn(async move {
        let result = run_resource_planning(&bus, Version(0), &erp, &partners, &bus, |e| match e {
            PlanningError::Command(e) if is_outdated_planning(&e) => {}
            e => eprintln!("warning: resource planning: {e}"),
        })
        .await;
        if let Err(e) = result {
            eprintln!("error: resource planning stopped: {e}");
        }
    });
}

fn spawn_customer_notices(commands: &CommandActorHandle, external: &ExternalSystems) {
    let source = commands.clone();
    let crm = external.crm_adapter();
    tokio::spawn(async move {
        let result = run_customer_notices(&source, Version(0), &crm, |notice, e| {
            eprintln!(
                "warning: customer of {} was not told about {}: {e}",
                notice.tenant_id, notice.ticket_id
            );
        })
        .await;
        if let Err(e) = result {
            eprintln!("error: customer notices stopped: {e}");
        }
    });
}

/// Тикет закрыли, пока ERP думала: проверять ресурсы и звать партнёра уже незачем.
fn is_outdated_planning(e: &CommandError) -> bool {
    matches!(
        e,
        CommandError::Rejected(ServiceError::Ticket(TicketError::Closed(_)))
    )
}

/// Дедлайн устарел, пока команда шла к агрегату: в тикете появилась активность,
/// его уже закрыли или напоминание больше не нужно. Это обычный исход, а не сбой.
fn is_outdated_deadline(e: &CommandError) -> bool {
    matches!(
        e,
        CommandError::Rejected(ServiceError::Ticket(
            TicketError::Closed(_)
                | TicketError::StillActive { .. }
                | TicketError::ReminderNotDue { .. }
                | TicketError::ReminderAlreadySent(_)
        ))
    )
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
    use api::Role;
    use api::{
        ChangesDto, CloseReason, CommandAcceptedDto, Cursor, ErrorDto, NotificationDto,
        NotificationKind, Recipient, ReminderKind, TenantDto, TicketDto, TicketStatus, UserDto,
    };
    use axum::{
        body::Body,
        http::{Request, StatusCode, header},
    };
    use external::crm::{CrmAccount, CrmCaseActivity};
    use http_body_util::BodyExt;
    use serde::de::DeserializeOwned;
    use ticketing::application::clock::{ManualClock, ScaledClock};
    use ticketing::domain::ticket::{INACTIVITY_TIMEOUT_SECS, STALE_REMINDER_AFTER_SECS};
    use ticketing::domain::ticket::{ResourceAvailability, TicketEvent};
    use tower::ServiceExt;

    /// Номера тикетов и пользователей из [`seed`].
    const ASSIGNED: usize = 1;
    const OPEN: usize = 2;
    const CLOSED: usize = 7;
    const AGENT_ALICE: usize = 1;
    const AGENT_CAROL: usize = 2;
    const CUSTOMER_BOB: usize = 3;
    const ADMIN_DAVE: usize = 4;
    const ACME: usize = 1;
    const GLOBEX: usize = 2;
    const DAY: u64 = 24 * 60 * 60;
    const WAIT: Duration = Duration::from_secs(5);

    /// Система с демо-данными, в которой всё уже разошлось: read-модель догнала seed,
    /// политика назначила скидку Acme.
    struct TestApp {
        router: Router,
        clock: ManualClock,
        commands: CommandActorHandle,
        external: ExternalSystems,
        /// Позиция после того, как всё разошлось.
        settled: Cursor,
    }

    async fn settled_app(long_poll: Duration) -> TestApp {
        let config = Config {
            long_poll,
            deadline_tick: Duration::from_secs(3600),
        };
        settled_app_with(config).await
    }

    async fn settled_app_with(config: Config) -> TestApp {
        let clock = ManualClock::default();
        let app = app(clock.clone(), config);
        seed(&app).await.unwrap();
        let router = app.router;
        wait_for_tenant(&router, ACME, |t| t.discount_percent == 10).await;
        let all: ChangesDto = get(&router, &api::changes(None)).await;
        TestApp {
            router,
            clock,
            commands: app.commands,
            external: app.external,
            settled: all.cursor(),
        }
    }

    /// Система без демо-данных: так сервер стартует без `--seed`.
    async fn empty_app(deadline_tick: Duration) -> TestApp {
        let clock = ManualClock::default();
        let app = app(
            clock.clone(),
            Config {
                long_poll: WAIT,
                deadline_tick,
            },
        );
        let router = app.router;
        let all: ChangesDto = get(&router, &api::changes(None)).await;
        TestApp {
            router,
            clock,
            commands: app.commands,
            external: app.external,
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

        /// Ждёт в общем журнале событие, для которого `pick` что-то возвращает.
        async fn wait_for_event<T>(&self, pick: impl Fn(&DomainEvent) -> Option<T>) -> T {
            use framework::cqrs::event_source::{EventSource, Subscription};
            let wait = async {
                let mut events = self.commands.subscribe(Version(0)).await.unwrap();
                while let Some(envelope) = events.next().await {
                    if let Some(found) = pick(&envelope.event) {
                        return found;
                    }
                }
                panic!("event stream closed");
            };
            tokio::time::timeout(WAIT, wait)
                .await
                .expect("expected event was recorded")
        }

        /// Ждёт, пока в истории обращений CRM наберётся `count` записей.
        async fn wait_for_crm_history(&self, count: usize) -> Vec<CrmCaseActivity> {
            let wait = async {
                loop {
                    let history = self.external.crm.activities();
                    if history.len() >= count {
                        return history;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            };
            tokio::time::timeout(WAIT, wait)
                .await
                .expect("CRM received the notices")
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

    const NEVER: Duration = Duration::from_secs(3600);

    /// Заводит через HTTP то, что раньше мог только seed: тенанта, агента и тикет.
    async fn acme_alice_and_ticket(app: &TestApp) {
        app.command(api::TENANTS, Some(r#"{"tenant_name":"Acme"}"#))
            .await;
        app.command(api::USERS, Some(r#"{"user_name":"Alice","role":"Agent"}"#))
            .await;
        app.command(
            api::TICKETS,
            Some(r#"{"category":"Billing","description":"Wrong invoice","tenant_id":1}"#),
        )
        .await;
    }

    #[tokio::test]
    async fn empty_system_is_filled_through_commands() {
        let app = empty_app(NEVER).await;
        let tickets: Vec<TicketDto> = app.get(api::TICKETS).await;
        assert_eq!(tickets, []);

        acme_alice_and_ticket(&app).await;
        let tenants: Vec<TenantDto> = app.get(api::TENANTS).await;
        assert_eq!(tenants[0].tenant_name, "Acme");
        assert_eq!(tenants[0].open_tickets, 1);
        let users: Vec<UserDto> = app.get(api::USERS).await;
        assert_eq!(users[0].user_name, "Alice");
        assert_eq!(users[0].role, Role::Agent);
        let tickets: Vec<TicketDto> = app.get(api::TICKETS).await;
        assert_eq!(tickets[0].category, "Billing");
        assert_eq!(tickets[0].tenant_id, 1);
        assert_eq!(tickets[0].status, TicketStatus::Open);
    }

    #[tokio::test]
    async fn creating_with_bad_data_is_rejected() {
        let app = empty_app(NEVER).await;
        let cases = [
            (
                api::TENANTS,
                r#"{"tenant_name":"  "}"#,
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                api::USERS,
                r#"{"user_name":"","role":"Agent"}"#,
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                api::TICKETS,
                r#"{"category":"Billing","description":"Invoice","tenant_id":99}"#,
                StatusCode::NOT_FOUND,
            ),
        ];
        for (uri, body, expected) in cases {
            let (status, _): (_, ErrorDto) = app.send("POST", uri, Some(body)).await;
            assert_eq!(status, expected, "POST {uri} {body}");
        }

        app.command(api::TENANTS, Some(r#"{"tenant_name":"Acme"}"#))
            .await;
        for body in [
            r#"{"category":"Billing","description":" ","tenant_id":1}"#,
            r#"{"category":" ","description":"Invoice","tenant_id":1}"#,
        ] {
            let (status, _): (_, ErrorDto) = app.send("POST", api::TICKETS, Some(body)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        }
    }

    #[tokio::test]
    async fn ticket_shows_its_deadline_and_how_it_was_closed() {
        let app = empty_app(NEVER).await;
        acme_alice_and_ticket(&app).await;
        let ticket: TicketDto = app.get(&api::ticket(1)).await;
        assert_eq!(ticket.last_activity, 0);
        assert_eq!(ticket.closes_at, Some(INACTIVITY_TIMEOUT_SECS));
        assert_eq!(ticket.close_reason, None);

        app.clock.advance(DAY);
        app.command(&api::assign_agent(1), Some(r#"{"agent_id":1}"#))
            .await;
        let ticket: TicketDto = app.get(&api::ticket(1)).await;
        assert_eq!(ticket.last_activity, DAY);
        assert_eq!(ticket.closes_at, Some(DAY + INACTIVITY_TIMEOUT_SECS));

        app.command(&api::close_ticket(1), None).await;
        let ticket: TicketDto = app.get(&api::ticket(1)).await;
        assert_eq!(ticket.close_reason, Some(CloseReason::Resolved));
        assert_eq!(ticket.closes_at, None);
    }

    #[tokio::test]
    async fn inbox_is_personal_for_agents_and_shared_by_admins() {
        let app = seeded_app().await;
        let of_alice: Vec<NotificationDto> = app.get(&api::notifications_of(AGENT_ALICE)).await;
        assert_eq!(of_alice.len(), 1);
        assert_eq!(of_alice[0].recipient, Recipient::Agent(AGENT_ALICE));
        assert_eq!(of_alice[0].kind, NotificationKind::Assigned);
        assert_eq!(of_alice[0].ticket_id, ASSIGNED);

        let of_dave: Vec<NotificationDto> = app.get(&api::notifications_of(ADMIN_DAVE)).await;
        assert_eq!(of_dave.len(), 9);
        assert!(of_dave.iter().all(|n| n.recipient == Recipient::Admins));

        let of_bob: Vec<NotificationDto> = app.get(&api::notifications_of(CUSTOMER_BOB)).await;
        assert_eq!(of_bob, []);
        let of_nobody: Vec<NotificationDto> = app.get(&api::notifications_of(99)).await;
        assert_eq!(of_nobody, []);
    }

    #[tokio::test]
    async fn long_polling_brings_new_notifications_once() {
        let app = seeded_app().await;
        let all: ChangesDto = app.get(&api::changes(None)).await;
        assert_eq!(all.notifications.len(), 10);

        app.command(
            &api::assign_agent(OPEN),
            Some(&format!(r#"{{"agent_id":{AGENT_CAROL}}}"#)),
        )
        .await;
        let changes: ChangesDto = app.get(&api::changes(Some(all.cursor()))).await;
        assert_eq!(changes.notifications.len(), 1);
        assert_eq!(
            changes.notifications[0].recipient,
            Recipient::Agent(AGENT_CAROL)
        );
        assert_eq!(changes.notifications[0].kind, NotificationKind::Assigned);
    }

    #[tokio::test]
    async fn reminders_reach_ticket_and_inbox_over_http() {
        let app = empty_app(Duration::from_millis(1)).await;
        acme_alice_and_ticket(&app).await;
        app.command(&api::assign_agent(1), Some(r#"{"agent_id":1}"#))
            .await;
        app.clock.advance(STALE_REMINDER_AFTER_SECS);

        let wait = async {
            let mut after = None;
            loop {
                let changes: ChangesDto = app.get(&api::changes(after)).await;
                if changes.tickets.iter().any(|t| t.reminders_sent == 1) {
                    return;
                }
                after = Some(changes.cursor());
            }
        };
        tokio::time::timeout(WAIT, wait)
            .await
            .expect("the reminder arrives");

        let inbox: Vec<NotificationDto> = app.get(&api::notifications_of(1)).await;
        assert_eq!(
            inbox.last().map(|n| n.kind),
            Some(NotificationKind::Reminder(ReminderKind::Stale))
        );
    }

    #[tokio::test]
    async fn tickets_are_closed_by_deadline_and_discount_follows() {
        let app = settled_app_with(Config {
            long_poll: WAIT,
            deadline_tick: Duration::from_millis(1),
        })
        .await;
        app.clock.advance(INACTIVITY_TIMEOUT_SECS);

        wait_for_tenant(&app.router, ACME, |t| {
            t.open_tickets == 0 && t.discount_percent == 0
        })
        .await;
        wait_for_tenant(&app.router, GLOBEX, |t| t.open_tickets == 0).await;
        let closed: Vec<TicketDto> = app.get("/api/tickets?status=Closed").await;
        assert_eq!(closed.len(), 9);
        // Седьмой тикет закрыли вручную ещё в seed, остальные закрылись сами.
        let auto_closed = closed
            .iter()
            .filter(|t| t.close_reason == Some(CloseReason::Inactivity))
            .count();
        assert_eq!(auto_closed, 8);
    }

    #[test]
    fn deadline_tick_shrinks_with_time_scale() {
        let tick = |scale| Config::for_time_scale(scale).deadline_tick;
        assert_eq!(tick(1), Duration::from_secs(1));
        assert_eq!(tick(60), Duration::from_secs(1));
        assert_eq!(tick(600), Duration::from_millis(100));
        assert_eq!(tick(86_400), Duration::from_millis(10));
        assert_eq!(tick(0), Duration::from_secs(1));
    }

    /// Без заглушек времени: те же часы и настройки, что у `--time-scale`.
    #[tokio::test]
    async fn accelerated_clock_closes_ticket_in_real_time() {
        const TEN_DAYS_PER_SECOND: u32 = 864_000;
        let clock = ScaledClock::starting_now(TEN_DAYS_PER_SECOND);
        let app = app(clock, Config::for_time_scale(TEN_DAYS_PER_SECOND));
        let router = app.router;
        for (uri, body) in [
            (api::TENANTS, r#"{"tenant_name":"Acme"}"#),
            (
                api::TICKETS,
                r#"{"category":"Billing","description":"Refund","tenant_id":1}"#,
            ),
        ] {
            let (status, _): (_, CommandAcceptedDto) = send(&router, "POST", uri, Some(body)).await;
            assert_eq!(status, StatusCode::OK);
        }

        let closed = tokio::time::timeout(WAIT, async {
            loop {
                let tickets: Vec<TicketDto> = get(&router, api::TICKETS).await;
                if let Some(ticket) = tickets.into_iter().find(|t| t.closed_at.is_some()) {
                    return ticket;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("ticket closes by deadline");

        assert_eq!(closed.close_reason, Some(CloseReason::Inactivity));
        assert_eq!(closed.reminders_sent, 2);
    }

    #[test]
    fn only_outdated_deadlines_are_expected_rejections() {
        let rejected = |e: TicketError| CommandError::Rejected(ServiceError::Ticket(e));
        assert!(is_outdated_deadline(&rejected(TicketError::StillActive {
            idle_secs: 0
        })));
        assert!(!is_outdated_deadline(&rejected(TicketError::NotFound)));
        assert!(!is_outdated_deadline(&CommandError::Unavailable));
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

    // --- Интеграция с CRM, ERP и партнёрами ---

    #[tokio::test]
    async fn crm_decides_who_may_open_tickets() {
        let app = empty_app(NEVER).await;
        app.command(api::TENANTS, Some(r#"{"tenant_name":"Acme"}"#))
            .await;
        let ticket = r#"{"category":"Billing","description":"Invoice","tenant_id":1}"#;
        app.command(api::TICKETS, Some(ticket)).await;

        let suspended = CrmAccount {
            acct_stat_cd: "S".to_owned(),
            ..integration::crm::new_account(
                ticketing::domain::tenant::TenantId(1),
                "Acme",
                ticketing::domain::sla::CustomerTier::Bronze,
            )
        };
        app.external.crm.upsert_account(suspended);
        let (status, error): (_, ErrorDto) = app.send("POST", api::TICKETS, Some(ticket)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            error.error,
            "tenant-1 cannot open tickets: the CRM account is not active"
        );

        app.external.crm.set_down(true);
        let (status, _): (_, ErrorDto) = app.send("POST", api::TICKETS, Some(ticket)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn customer_hears_about_ticket_through_crm() {
        let app = empty_app(NEVER).await;
        app.clock.advance(1_000);
        app.command(
            api::TENANTS,
            Some(r#"{"tenant_name":"Acme","tier":"Gold"}"#),
        )
        .await;
        let ticket = r#"{"category":"Billing","description":"Invoice","tenant_id":1}"#;
        app.command(api::TICKETS, Some(ticket)).await;
        app.clock.advance(DAY + 1);
        app.command(&api::close_ticket(1), None).await;

        let history = app.wait_for_crm_history(2).await;
        assert_eq!(
            history[0],
            CrmCaseActivity {
                acct_no: "ACC-000001".to_owned(),
                case_ref: "TKT-1".to_owned(),
                case_stat: "NEW".to_owned(),
                sla_met_flag: String::new(),
                due_ts_ms: (1_000 + DAY) * 1_000,
                actv_ts_ms: 1_000_000,
            }
        );
        assert_eq!(history[1].case_stat, "RESOLVED");
        assert_eq!(
            history[1].sla_met_flag, "N",
            "Gold has one day, it took longer"
        );
    }

    #[tokio::test]
    async fn ticket_without_parts_goes_to_partner_and_with_parts_stays() {
        let app = seeded_app().await;
        // Принтер (тикет 4) — Hardware, запчастей нет; VPN (тикет 1) — Network, оборудование есть.
        let handed = app
            .wait_for_event(|event| match event {
                DomainEvent::Ticket(TicketEvent::HandedToPartner(e)) => Some(e.clone()),
                _ => None,
            })
            .await;
        assert_eq!(handed.ticket_id.0, 4);
        assert_eq!(handed.partner_id, FIXIT);
        let vpn = app
            .wait_for_event(|event| match event {
                DomainEvent::Ticket(TicketEvent::ResourcesChecked(e)) if e.ticket_id.0 == 1 => {
                    Some(e.availability)
                }
                _ => None,
            })
            .await;
        assert_eq!(vpn, ResourceAvailability::Available);
    }

    // --- Open-Host Service для партнёров ---

    use partner_api::{ErrorCode, PartnerError, PartnerTicket, PartnerTicketStatus};

    /// Запрос к Open-Host Service от имени партнёра с токеном `token`.
    async fn as_partner<T: DeserializeOwned>(
        app: &TestApp,
        token: Option<&str>,
        method: &str,
        uri: &str,
        body: Option<&str>,
    ) -> (StatusCode, T) {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, partner_api::bearer(token));
        }
        let request = request
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_owned())))
            .unwrap();
        let response = app.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    /// Ждёт, пока партнёру FixIt передадут тикет про принтер из seed.
    async fn fixit_tickets(app: &TestApp) -> Vec<PartnerTicket> {
        let wait = async {
            loop {
                let (status, tickets): (_, Vec<PartnerTicket>) =
                    as_partner(app, Some(FIXIT_TOKEN), "GET", partner_api::TICKETS, None).await;
                assert_eq!(status, StatusCode::OK);
                if !tickets.is_empty() {
                    return tickets;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        };
        tokio::time::timeout(WAIT, wait)
            .await
            .expect("printer ticket was handed to FixIt")
    }

    fn update(status: &str, comment: &str) -> String {
        format!(r#"{{"status":"{status}","comment":"{comment}"}}"#)
    }

    #[tokio::test]
    async fn partner_sees_only_tickets_handed_to_it() {
        let app = seeded_app().await;
        let tickets = fixit_tickets(&app).await;
        assert_eq!(
            tickets,
            [PartnerTicket {
                ticket_id: 4,
                category: "Hardware".to_owned(),
                description: "Printer on floor 3 is offline".to_owned(),
                customer: "Acme".to_owned(),
                status: PartnerTicketStatus::Open,
                handed_over_at: 0,
                due_at: DAY,
                closed_at: None,
                updates: Vec::new(),
            }]
        );
        let (status, one): (_, PartnerTicket) = as_partner(
            &app,
            Some(FIXIT_TOKEN),
            "GET",
            &partner_api::ticket(4),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(one, tickets[0]);

        let (_, netco): (_, Vec<PartnerTicket>) =
            as_partner(&app, Some(NETCO_TOKEN), "GET", partner_api::TICKETS, None).await;
        assert_eq!(netco, []);
        for (token, ticket) in [(NETCO_TOKEN, 4), (FIXIT_TOKEN, 1), (FIXIT_TOKEN, 99)] {
            let (status, error): (_, PartnerError) =
                as_partner(&app, Some(token), "GET", &partner_api::ticket(ticket), None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{token} ticket {ticket}");
            assert_eq!(error.code, ErrorCode::NotFound);
        }
    }

    #[tokio::test]
    async fn partner_api_requires_a_known_token() {
        let app = empty_app(NEVER).await;
        for token in [None, Some("stolen-token")] {
            let request = Request::builder()
                .uri(partner_api::TICKETS)
                .header(
                    header::AUTHORIZATION,
                    token.map_or(String::new(), partner_api::bearer),
                )
                .body(Body::empty())
                .unwrap();
            let response = app.router.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{token:?}");
            assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");
        }
        let (status, error): (_, PartnerError) =
            as_partner(&app, None, "GET", partner_api::TICKETS, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(error.code, ErrorCode::Unauthorized);
    }

    #[tokio::test]
    async fn partner_reports_progress_and_resolves_ticket() {
        let app = seeded_app().await;
        fixit_tickets(&app).await;
        let uri = partner_api::ticket_updates(4);

        app.clock.advance(2 * DAY);
        let body = update("in_progress", "Replacement fuser ordered");
        let (status, ticket): (_, PartnerTicket) =
            as_partner(&app, Some(FIXIT_TOKEN), "POST", &uri, Some(&body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(ticket.status, PartnerTicketStatus::Open);
        assert_eq!(ticket.updates.len(), 1);
        assert_eq!(ticket.updates[0].at, 2 * DAY);
        // Отчёт партнёра — активность: автозакрытие отодвинулось.
        let internal: TicketDto = app.get(&api::ticket(4)).await;
        assert_eq!(internal.closes_at, Some(2 * DAY + INACTIVITY_TIMEOUT_SECS));

        let (status, error): (_, PartnerError) = as_partner(
            &app,
            Some(FIXIT_TOKEN),
            "POST",
            &uri,
            Some(&update("in_progress", " ")),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error.code, ErrorCode::Invalid);

        let body = update("resolved", "Fuser replaced, printer works");
        let (status, ticket): (_, PartnerTicket) =
            as_partner(&app, Some(FIXIT_TOKEN), "POST", &uri, Some(&body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(ticket.status, PartnerTicketStatus::Resolved);
        assert_eq!(ticket.closed_at, Some(2 * DAY));
        assert_eq!(ticket.updates.len(), 2);
        let internal: TicketDto = app.get(&api::ticket(4)).await;
        assert_eq!(internal.status, TicketStatus::Closed);
        assert_eq!(internal.close_reason, Some(CloseReason::Resolved));

        let (status, error): (_, PartnerError) = as_partner(
            &app,
            Some(FIXIT_TOKEN),
            "POST",
            &uri,
            Some(&update("resolved", "")),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error.code, ErrorCode::Closed);

        // Клиент узнал о решении через CRM: срок Gold — сутки, решили за двое.
        let history = app.wait_for_crm_history(11).await;
        let closing = history
            .iter()
            .find(|a| a.case_ref == "TKT-4" && a.case_stat == "RESOLVED")
            .expect("customer is told about the resolution");
        assert_eq!(closing.sla_met_flag, "N");
    }

    #[tokio::test]
    async fn other_partner_cannot_report_on_foreign_ticket() {
        let app = seeded_app().await;
        fixit_tickets(&app).await;
        let body = update("resolved", "Not ours, but closing");
        let (status, error): (_, PartnerError) = as_partner(
            &app,
            Some(NETCO_TOKEN),
            "POST",
            &partner_api::ticket_updates(4),
            Some(&body),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(error.code, ErrorCode::NotFound);
        let internal: TicketDto = app.get(&api::ticket(4)).await;
        assert_eq!(internal.status, TicketStatus::Open);
    }
}
