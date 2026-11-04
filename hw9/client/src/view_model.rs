//! ViewModel: состояние формы в реактивных сигналах, проверка доступности команд и сами команды.
//!
//! Представление читает свойства (`tickets`, `selected`, `can_close`, ...) внутри реактивных
//! замыканий и перерисовывается, когда меняются сигналы.
//!
//! Состояние — копия read-модели сервера ([`Replica`]), её обновляет цикл long polling
//! [`sync_forever`](TicketViewModel::sync_forever). Команды состояние не трогают: сервер
//! отвечает версией, и модель остаётся занятой, пока поток изменений не дойдёт до этой версии.
//! Так клиент видит результат своей команды тем же путём, что и изменения других клиентов.

use std::sync::Arc;
use std::time::Duration;

use api::{CommandAcceptedDto, Role, TenantDto, TicketDto, UserDto};
use leptos::prelude::*;

use crate::model::TicketRules;
use crate::model::backoff::Backoff;
use crate::model::http::HttpClient;
use crate::model::replica::Replica;
use crate::model::ticket_api::{ApiError, TicketApiService, TicketQuery};

pub struct TicketViewModel<H: 'static> {
    api: StoredValue<Arc<TicketApiService<H>>>,
    replica: RwSignal<Replica>,
    selected_ticket: RwSignal<Option<usize>>,
    selected_user: RwSignal<Option<usize>>,
    /// Команда отправлена, ответа ещё нет.
    sending: RwSignal<bool>,
    /// Версия своей последней команды, до которой копия ещё не дошла.
    pending: RwSignal<Option<u64>>,
    /// Последний запрос изменений прошёл успешно.
    connected: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    query: RwSignal<Option<TicketQuery>>,
    query_results: RwSignal<Vec<TicketDto>>,
    query_error: RwSignal<Option<String>>,
}

// Все поля — дескрипторы сигналов, поэтому модель копируется в замыкания представления
// без ограничений на `H`.
impl<H> Clone for TicketViewModel<H> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<H> Copy for TicketViewModel<H> {}

impl<H: HttpClient + Send + Sync + 'static> TicketViewModel<H> {
    pub fn new(api: TicketApiService<H>) -> Self {
        Self {
            api: StoredValue::new(Arc::new(api)),
            replica: RwSignal::new(Replica::default()),
            selected_ticket: RwSignal::new(None),
            selected_user: RwSignal::new(None),
            sending: RwSignal::new(false),
            pending: RwSignal::new(None),
            connected: RwSignal::new(false),
            error: RwSignal::new(None),
            query: RwSignal::new(None),
            query_results: RwSignal::new(Vec::new()),
            query_error: RwSignal::new(None),
        }
    }

    // --- Свойства для привязки ---

    pub fn tickets(&self) -> Vec<TicketDto> {
        self.replica.with(|r| r.tickets().cloned().collect())
    }

    pub fn users(&self) -> Vec<UserDto> {
        self.replica.with(|r| r.users().cloned().collect())
    }

    pub fn agents(&self) -> Vec<UserDto> {
        self.replica.with(|r| {
            r.users()
                .filter(|u| u.role == Role::Agent)
                .cloned()
                .collect()
        })
    }

    pub fn tenants(&self) -> Vec<TenantDto> {
        self.replica.with(|r| r.tenants().cloned().collect())
    }

    pub fn selected(&self) -> Option<TicketDto> {
        let id = self.selected_ticket.get()?;
        self.replica.with(|r| r.ticket(id).cloned())
    }

    pub fn is_selected(&self, ticket_id: usize) -> bool {
        self.selected_ticket.get() == Some(ticket_id)
    }

    pub fn selected_user_id(&self) -> Option<usize> {
        self.selected_user.get()
    }

    /// Команда в полёте или её результат ещё не пришёл с сервера.
    pub fn is_busy(&self) -> bool {
        self.sending.get() || self.pending.get().is_some()
    }

    pub fn is_connected(&self) -> bool {
        self.connected.get()
    }

    /// До какой версии сервера дошла копия.
    pub fn version(&self) -> Option<u64> {
        self.replica.with(Replica::version)
    }

    pub fn error(&self) -> Option<String> {
        self.error.get()
    }

    pub fn user_name(&self, user_id: usize) -> Option<String> {
        self.replica
            .with(|r| r.user(user_id).map(|u| u.user_name.clone()))
    }

    pub fn tenant(&self, tenant_id: usize) -> Option<TenantDto> {
        self.replica.with(|r| r.tenant(tenant_id).cloned())
    }

    pub fn query(&self) -> Option<TicketQuery> {
        self.query.get()
    }

    pub fn query_results(&self) -> Vec<TicketDto> {
        self.query_results.get()
    }

    pub fn query_error(&self) -> Option<String> {
        self.query_error.get()
    }

    pub fn select_ticket(&self, ticket_id: usize) {
        self.selected_ticket.set(Some(ticket_id));
        self.error.set(None);
    }

    pub fn select_user(&self, user_id: Option<usize>) {
        self.selected_user.set(user_id);
    }

    // --- Доступность команд (CanExecute) ---

    pub fn can_open(&self) -> bool {
        !self.is_busy() && self.selected().is_some_and(|t| t.validate_open().is_ok())
    }

    pub fn can_close(&self) -> bool {
        !self.is_busy() && self.selected().is_some_and(|t| t.validate_close().is_ok())
    }

    pub fn can_assign(&self) -> bool {
        !self.is_busy() && self.assignment().is_some_and(|r| r.is_ok())
    }

    /// Почему выбранного пользователя нельзя назначить на выбранный тикет.
    pub fn assign_hint(&self) -> Option<String> {
        self.assignment()?.err()
    }

    fn assignment(&self) -> Option<Result<(TicketDto, UserDto), String>> {
        let ticket = self.selected()?;
        let user_id = self.selected_user.get()?;
        let user = self.replica.with(|r| r.user(user_id).cloned())?;
        Some(match ticket.validate_assign(&user) {
            Ok(()) => Ok((ticket, user)),
            Err(e) => Err(e.to_string()),
        })
    }

    // --- Синхронизация с сервером ---

    /// Бесконечный цикл long polling. После обрыва связи пробует снова с нарастающей паузой;
    /// `sleep` — таймер платформы (в браузере `setTimeout`).
    pub async fn sync_forever<F: Future<Output = ()>>(&self, sleep: impl Fn(Duration) -> F) {
        let mut backoff = Backoff::default();
        loop {
            match self.poll_once().await {
                Ok(()) => backoff.succeeded(),
                Err(_) => sleep(backoff.failed()).await,
            }
        }
    }

    /// Один шаг long polling: получает изменения после текущей позиции и применяет их.
    /// Если изменились данные, а в панели запросов выбран запрос, обновляет и его.
    pub async fn poll_once(&self) -> Result<(), ApiError> {
        let api = self.api.get_value();
        let after = self.replica.with_untracked(Replica::cursor);
        let changes = match api.changes(after).await {
            Ok(changes) => changes,
            Err(e) => {
                self.connected.set(false);
                return Err(e);
            }
        };
        self.connected.set(true);
        let full = changes.full;
        let changed = full
            || !(changes.tickets.is_empty()
                && changes.tenants.is_empty()
                && changes.users.is_empty());
        self.replica.update(|r| r.apply(changes));

        let reached = self
            .replica
            .with_untracked(|r| match self.pending.get_untracked() {
                Some(version) => r.has_reached(version),
                None => false,
            });
        // После полной перезагрузки версия своей команды из прошлого запуска уже не придёт.
        if full || reached {
            self.pending.set(None);
        }
        let selected_exists = self
            .selected_ticket
            .get_untracked()
            .is_some_and(|id| self.replica.with_untracked(|r| r.ticket(id).is_some()));
        if !selected_exists {
            let first = self
                .replica
                .with_untracked(|r| r.tickets().next().map(|t| t.ticket_id));
            self.selected_ticket.set(first);
        }
        if changed {
            self.refresh_query().await;
        }
        Ok(())
    }

    // --- Команды ---

    pub async fn open_ticket(&self) {
        if !self.can_open() {
            return;
        }
        let Some(ticket) = self.selected() else {
            return;
        };
        let api = self.api.get_value();
        self.run(api.open_ticket(&ticket)).await;
    }

    pub async fn close_ticket(&self) {
        if !self.can_close() {
            return;
        }
        let Some(ticket) = self.selected() else {
            return;
        };
        let api = self.api.get_value();
        self.run(api.close_ticket(&ticket)).await;
    }

    pub async fn assign_agent(&self) {
        if !self.can_assign() {
            return;
        }
        let Some(Ok((ticket, agent))) = self.assignment() else {
            return;
        };
        let api = self.api.get_value();
        self.run(api.assign_agent(&ticket, &agent)).await;
    }

    /// Выбирает запрос в панели запросов и выполняет его.
    pub async fn run_query(&self, query: TicketQuery) {
        self.query.set(Some(query));
        self.refresh_query().await;
    }

    async fn refresh_query(&self) {
        let Some(query) = self.query.get_untracked() else {
            return;
        };
        let result = self.api.get_value().query(query).await;
        // Пока ответ шёл, могли выбрать другой запрос: тогда этот ответ уже не нужен.
        if self.query.get_untracked() != Some(query) {
            return;
        }
        match result {
            Ok(tickets) => {
                self.query_results.set(tickets);
                self.query_error.set(None);
            }
            Err(e) => self.query_error.set(Some(e.to_string())),
        }
    }

    async fn run(&self, command: impl Future<Output = Result<CommandAcceptedDto, ApiError>>) {
        self.sending.set(true);
        self.error.set(None);
        match command.await {
            Ok(accepted) => {
                let seen = self
                    .replica
                    .with_untracked(|r| r.has_reached(accepted.version));
                if !seen {
                    self.pending.set(Some(accepted.version));
                }
            }
            Err(e) => self.error.set(Some(e.to_string())),
        }
        self.sending.set(false);
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;

    use super::*;
    use crate::model::fixtures::{changes, ticket, user};
    use crate::model::http::HttpRequest;
    use crate::model::mock_http::MockHttpClient;
    use api::{ChangesDto, TicketStatus};

    const EPOCH: u64 = 1;
    const LOADED: u64 = 10;

    fn tenant_dto(open_tickets: usize, discount_percent: u8) -> TenantDto {
        TenantDto {
            tenant_id: 1,
            tenant_name: "Acme".to_owned(),
            discount_percent,
            open_tickets,
        }
    }

    fn delta(version: u64, tickets: Vec<TicketDto>) -> ChangesDto {
        changes(EPOCH, version, false, tickets)
    }

    /// Модель после первого запроса изменений: тикет 1 открыт и выбран, тикет 2 закрыт.
    fn loaded() -> (TicketViewModel<MockHttpClient>, MockHttpClient) {
        let http = MockHttpClient::default();
        http.respond_json(
            200,
            &ChangesDto {
                users: vec![
                    user(1, "Alice", Role::Agent),
                    user(2, "Bob", Role::Customer),
                ],
                tenants: vec![tenant_dto(6, 10)],
                ..changes(
                    EPOCH,
                    LOADED,
                    true,
                    vec![
                        ticket(1, TicketStatus::Open),
                        ticket(2, TicketStatus::Closed),
                    ],
                )
            },
        );
        let vm = TicketViewModel::new(TicketApiService::new(http.clone()));
        block_on(vm.poll_once()).unwrap();
        (vm, http)
    }

    fn status(vm: &TicketViewModel<MockHttpClient>, id: usize) -> TicketStatus {
        let tickets = vm.tickets();
        tickets.iter().find(|t| t.ticket_id == id).unwrap().status
    }

    fn last_request(http: &MockHttpClient) -> HttpRequest {
        http.requests().pop().unwrap()
    }

    #[test]
    fn first_poll_fills_state_and_selects_first_ticket() {
        let (vm, http) = loaded();
        assert_eq!(vm.tickets().len(), 2);
        assert_eq!(vm.users().len(), 2);
        assert_eq!(vm.agents().len(), 1);
        assert_eq!(vm.tenant(1).unwrap().discount_percent, 10);
        assert_eq!(vm.selected().unwrap().ticket_id, 1);
        assert_eq!(vm.version(), Some(LOADED));
        assert!(vm.is_connected());
        assert_eq!(http.requests(), [HttpRequest::get("/api/changes")]);
        assert!(!vm.is_busy());
    }

    #[test]
    fn next_poll_continues_from_cursor_and_applies_changes_of_others() {
        let (vm, http) = loaded();
        let mut closed = ticket(1, TicketStatus::Closed);
        closed.closed_at = Some(100);
        http.respond_json(
            200,
            &ChangesDto {
                tenants: vec![tenant_dto(5, 0)],
                ..delta(11, vec![closed])
            },
        );

        block_on(vm.poll_once()).unwrap();

        assert_eq!(last_request(&http).path, "/api/changes?epoch=1&since=10");
        assert_eq!(status(&vm, 1), TicketStatus::Closed);
        assert_eq!(vm.tenant(1).unwrap().discount_percent, 0);
        assert_eq!(vm.version(), Some(11));
        assert!(vm.can_open());
    }

    #[test]
    fn lost_connection_is_reported_and_state_kept() {
        let (vm, http) = loaded();
        http.fail("connection refused");
        assert!(block_on(vm.poll_once()).is_err());
        assert!(!vm.is_connected());
        assert_eq!(vm.tickets().len(), 2);

        http.respond_json(200, &delta(LOADED, vec![]));
        block_on(vm.poll_once()).unwrap();
        assert!(vm.is_connected());
    }

    #[test]
    fn commands_follow_ticket_status() {
        let (vm, _) = loaded();
        assert!(vm.can_close());
        assert!(!vm.can_open());

        vm.select_ticket(2);
        assert!(!vm.can_close());
        assert!(vm.can_open());
    }

    #[test]
    fn nothing_is_executable_without_selection_or_while_busy() {
        let vm = TicketViewModel::new(TicketApiService::new(MockHttpClient::default()));
        assert!(!vm.can_open() && !vm.can_close() && !vm.can_assign());

        let (vm, _) = loaded();
        vm.select_user(Some(1));
        vm.pending.set(Some(LOADED + 1));
        assert!(!vm.can_open() && !vm.can_close() && !vm.can_assign());
    }

    #[test]
    fn assign_requires_an_agent() {
        let (vm, _) = loaded();
        assert!(!vm.can_assign());
        assert_eq!(vm.assign_hint(), None);

        vm.select_user(Some(2));
        assert!(!vm.can_assign());
        assert_eq!(
            vm.assign_hint().as_deref(),
            Some("Bob has role Customer, only agents can be assigned")
        );

        vm.select_user(Some(1));
        assert!(vm.can_assign());
        assert_eq!(vm.assign_hint(), None);
    }

    #[test]
    fn command_keeps_model_busy_until_its_version_arrives() {
        let (vm, http) = loaded();
        http.respond_json(200, &CommandAcceptedDto { version: 12 });

        block_on(vm.close_ticket());

        assert_eq!(
            last_request(&http),
            HttpRequest::post("/api/tickets/1/close", None)
        );
        // Сервер принял команду, но состояние приходит только через поток изменений.
        assert!(vm.is_busy());
        assert_eq!(status(&vm, 1), TicketStatus::Open);

        // Сначала пришло чужое изменение, своё ещё нет.
        http.respond_json(200, &delta(11, vec![ticket(2, TicketStatus::Open)]));
        block_on(vm.poll_once()).unwrap();
        assert!(vm.is_busy());

        http.respond_json(
            200,
            &ChangesDto {
                tenants: vec![tenant_dto(5, 0)],
                ..delta(12, vec![ticket(1, TicketStatus::Closed)])
            },
        );
        block_on(vm.poll_once()).unwrap();
        assert!(!vm.is_busy());
        assert_eq!(status(&vm, 1), TicketStatus::Closed);
        assert_eq!(vm.tenant(1).unwrap().discount_percent, 0);
        assert!(vm.can_open());
    }

    #[test]
    fn command_already_seen_does_not_block() {
        let (vm, http) = loaded();
        http.respond_json(200, &CommandAcceptedDto { version: LOADED });
        block_on(vm.close_ticket());
        assert!(!vm.is_busy());
    }

    #[test]
    fn server_restart_releases_pending_command() {
        let (vm, http) = loaded();
        http.respond_json(200, &CommandAcceptedDto { version: 12 });
        block_on(vm.close_ticket());

        http.respond_json(
            200,
            &changes(EPOCH + 1, 3, true, vec![ticket(1, TicketStatus::Open)]),
        );
        block_on(vm.poll_once()).unwrap();

        assert!(!vm.is_busy());
        assert_eq!(vm.tickets().len(), 1);
        assert_eq!(vm.version(), Some(3));
    }

    #[test]
    fn selection_moves_to_first_ticket_when_selected_one_disappears() {
        let (vm, http) = loaded();
        vm.select_ticket(2);
        http.respond_json(
            200,
            &changes(EPOCH + 1, 1, true, vec![ticket(1, TicketStatus::Open)]),
        );
        block_on(vm.poll_once()).unwrap();
        assert_eq!(vm.selected().unwrap().ticket_id, 1);
    }

    #[test]
    fn server_error_is_shown_and_state_kept() {
        let (vm, http) = loaded();
        http.respond_error(409, "ticket-1 is closed");

        block_on(vm.close_ticket());

        assert_eq!(status(&vm, 1), TicketStatus::Open);
        assert_eq!(
            vm.error().as_deref(),
            Some("server rejected the request (409): ticket-1 is closed")
        );
        assert!(!vm.is_busy());
        assert!(vm.can_close());
    }

    #[test]
    fn unavailable_command_sends_nothing() {
        let (vm, http) = loaded();
        block_on(vm.open_ticket());
        vm.select_user(Some(2));
        block_on(vm.assign_agent());
        assert_eq!(http.requests().len(), 1);
    }

    #[test]
    fn assign_sends_selected_agent() {
        let (vm, http) = loaded();
        vm.select_user(Some(1));
        http.respond_json(200, &CommandAcceptedDto { version: 11 });

        block_on(vm.assign_agent());

        let request = last_request(&http);
        assert_eq!(request.path, "/api/tickets/1/assign");
        assert!(vm.is_busy());
    }

    #[test]
    fn query_results_follow_changes() {
        let (vm, http) = loaded();
        http.respond_json(200, &[ticket(2, TicketStatus::Closed)]);
        block_on(vm.run_query(TicketQuery::ClosedWithin(7)));
        assert_eq!(last_request(&http).path, "/api/tickets/closed?days=7");
        assert_eq!(vm.query(), Some(TicketQuery::ClosedWithin(7)));
        assert_eq!(vm.query_results().len(), 1);

        // Данные изменились — выбранный запрос выполняется заново.
        http.respond_json(200, &delta(11, vec![ticket(1, TicketStatus::Closed)]))
            .respond_json(
                200,
                &[
                    ticket(1, TicketStatus::Closed),
                    ticket(2, TicketStatus::Closed),
                ],
            );
        block_on(vm.poll_once()).unwrap();
        assert_eq!(vm.query_results().len(), 2);

        // Пустой ответ long polling запрос не трогает.
        http.respond_json(200, &delta(11, vec![]));
        block_on(vm.poll_once()).unwrap();
        assert_eq!(http.requests().len(), 5);
    }

    #[test]
    fn query_error_is_shown_separately() {
        let (vm, http) = loaded();
        http.respond_error(500, "storage error");
        block_on(vm.run_query(TicketQuery::ActiveOfAgent(1)));
        assert_eq!(
            vm.query_error().as_deref(),
            Some("server rejected the request (500): storage error")
        );
        assert_eq!(vm.error(), None);
    }
}
