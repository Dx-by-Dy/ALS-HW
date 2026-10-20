//! ViewModel: состояние формы в реактивных сигналах, проверка доступности команд и сами команды.
//!
//! Представление читает свойства (`tickets`, `selected`, `can_close`, ...) внутри реактивных
//! замыканий и перерисовывается, когда меняются сигналы. Команды асинхронные: пока запрос
//! в полёте, `is_busy` блокирует остальные команды, а интерфейс продолжает отвечать.

use std::sync::Arc;

use leptos::prelude::*;

use crate::model::http::HttpClient;
use crate::model::ticket_api::{ApiError, TicketApiService};
use crate::model::{Tenant, TenantId, Ticket, TicketId, User, UserId};

pub struct TicketViewModel<H: 'static> {
    api: StoredValue<Arc<TicketApiService<H>>>,
    tickets: RwSignal<Vec<Ticket>>,
    users: RwSignal<Vec<User>>,
    tenants: RwSignal<Vec<Tenant>>,
    selected_ticket: RwSignal<Option<TicketId>>,
    selected_user: RwSignal<Option<UserId>>,
    busy: RwSignal<bool>,
    error: RwSignal<Option<String>>,
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
            tickets: RwSignal::new(Vec::new()),
            users: RwSignal::new(Vec::new()),
            tenants: RwSignal::new(Vec::new()),
            selected_ticket: RwSignal::new(None),
            selected_user: RwSignal::new(None),
            busy: RwSignal::new(false),
            error: RwSignal::new(None),
        }
    }

    // --- Свойства для привязки ---

    pub fn tickets(&self) -> Vec<Ticket> {
        self.tickets.get()
    }

    pub fn users(&self) -> Vec<User> {
        self.users.get()
    }

    pub fn tenants(&self) -> Vec<Tenant> {
        self.tenants.get()
    }

    pub fn selected(&self) -> Option<Ticket> {
        let id = self.selected_ticket.get()?;
        self.tickets
            .with(|tickets| tickets.iter().find(|t| t.ticket_id == id).cloned())
    }

    pub fn is_selected(&self, ticket_id: TicketId) -> bool {
        self.selected_ticket.get() == Some(ticket_id)
    }

    pub fn selected_user_id(&self) -> Option<UserId> {
        self.selected_user.get()
    }

    pub fn is_busy(&self) -> bool {
        self.busy.get()
    }

    pub fn error(&self) -> Option<String> {
        self.error.get()
    }

    pub fn user_name(&self, user_id: UserId) -> Option<String> {
        self.users.with(|users| {
            users
                .iter()
                .find(|u| u.user_id == user_id)
                .map(|u| u.user_name.clone())
        })
    }

    pub fn tenant(&self, tenant_id: TenantId) -> Option<Tenant> {
        self.tenants
            .with(|tenants| tenants.iter().find(|t| t.tenant_id == tenant_id).cloned())
    }

    pub fn select_ticket(&self, ticket_id: TicketId) {
        self.selected_ticket.set(Some(ticket_id));
        self.error.set(None);
    }

    pub fn select_user(&self, user_id: Option<UserId>) {
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

    fn assignment(&self) -> Option<Result<(Ticket, User), String>> {
        let ticket = self.selected()?;
        let user_id = self.selected_user.get()?;
        let user = self
            .users
            .with(|users| users.iter().find(|u| u.user_id == user_id).cloned())?;
        Some(match ticket.validate_assign(&user) {
            Ok(()) => Ok((ticket, user)),
            Err(e) => Err(e.to_string()),
        })
    }

    // --- Команды ---

    /// Загружает тикеты, пользователей и тенантов; выбирает первый тикет, если ничего не выбрано.
    pub async fn load(&self) {
        let api = self.api.get_value();
        self.run(async {
            let tickets = api.tickets().await?;
            self.users.set(api.users().await?);
            self.tenants.set(api.tenants().await?);
            if self.selected_ticket.get_untracked().is_none() {
                self.selected_ticket
                    .set(tickets.first().map(|t| t.ticket_id));
            }
            self.tickets.set(tickets);
            Ok(())
        })
        .await;
    }

    pub async fn open_ticket(&self) {
        if !self.can_open() {
            return;
        }
        let Some(ticket) = self.selected() else {
            return;
        };
        let api = self.api.get_value();
        self.run(async {
            let opened = api.open_ticket(&ticket).await?;
            self.sync_after_status_change(&api, opened).await
        })
        .await;
    }

    pub async fn close_ticket(&self) {
        if !self.can_close() {
            return;
        }
        let Some(ticket) = self.selected() else {
            return;
        };
        let api = self.api.get_value();
        self.run(async {
            let closed = api.close_ticket(&ticket).await?;
            self.sync_after_status_change(&api, closed).await
        })
        .await;
    }

    pub async fn assign_agent(&self) {
        if !self.can_assign() {
            return;
        }
        let Some(Ok((ticket, agent))) = self.assignment() else {
            return;
        };
        let api = self.api.get_value();
        self.run(async {
            let assigned = api.assign_agent(&ticket, &agent).await?;
            self.replace_ticket(assigned);
            Ok(())
        })
        .await;
    }

    /// Число открытых тикетов изменилось: синхронизируем тикет и пересчитываем скидку тенанта.
    async fn sync_after_status_change(
        &self,
        api: &TicketApiService<H>,
        ticket: Ticket,
    ) -> Result<(), ApiError> {
        let tenant_id = ticket.tenant_id;
        self.replace_ticket(ticket);
        let tenant = api.apply_discount(tenant_id).await?;
        self.tenants.update(|tenants| {
            if let Some(t) = tenants.iter_mut().find(|t| t.tenant_id == tenant_id) {
                *t = tenant;
            }
        });
        Ok(())
    }

    fn replace_ticket(&self, ticket: Ticket) {
        self.tickets.update(|tickets| {
            if let Some(t) = tickets.iter_mut().find(|t| t.ticket_id == ticket.ticket_id) {
                *t = ticket;
            }
        });
    }

    async fn run(&self, command: impl Future<Output = Result<(), ApiError>>) {
        self.busy.set(true);
        self.error.set(None);
        if let Err(e) = command.await {
            self.error.set(Some(e.to_string()));
        }
        self.busy.set(false);
    }
}

#[cfg(test)]
mod tests {
    use api::{TenantDto, TicketDto, UserDto};
    use futures::executor::block_on;

    use super::*;
    use crate::model::http::HttpRequest;
    use crate::model::mock_http::MockHttpClient;
    use crate::model::{Role, TicketStatus};

    fn ticket_dto(id: usize, status: TicketStatus) -> TicketDto {
        TicketDto {
            ticket_id: id,
            category: "Technical Issue".to_owned(),
            description: format!("Problem {id}"),
            status,
            assigned_agent: None,
            tenant_id: 1,
        }
    }

    fn tenant_dto(open_tickets: usize, discount_percent: u8) -> TenantDto {
        TenantDto {
            tenant_id: 1,
            tenant_name: "Acme".to_owned(),
            discount_percent,
            open_tickets,
        }
    }

    /// Модель после `load`: тикет 1 открыт и выбран, тикет 2 закрыт.
    fn loaded() -> (TicketViewModel<MockHttpClient>, MockHttpClient) {
        let http = MockHttpClient::default();
        http.respond_json(
            200,
            &[
                ticket_dto(1, TicketStatus::Open),
                ticket_dto(2, TicketStatus::Closed),
            ],
        )
        .respond_json(
            200,
            &[
                UserDto {
                    user_id: 1,
                    user_name: "Alice".to_owned(),
                    role: Role::Agent,
                },
                UserDto {
                    user_id: 2,
                    user_name: "Bob".to_owned(),
                    role: Role::Customer,
                },
            ],
        )
        .respond_json(200, &[tenant_dto(6, 10)]);
        let vm = TicketViewModel::new(TicketApiService::new(http.clone()));
        block_on(vm.load());
        (vm, http)
    }

    fn status(vm: &TicketViewModel<MockHttpClient>, id: usize) -> TicketStatus {
        let tickets = vm.tickets();
        tickets
            .iter()
            .find(|t| t.ticket_id == TicketId(id))
            .unwrap()
            .status
    }

    #[test]
    fn load_fills_state_and_selects_first_ticket() {
        let (vm, http) = loaded();
        assert_eq!(vm.tickets().len(), 2);
        assert_eq!(vm.users().len(), 2);
        assert_eq!(vm.tenant(TenantId(1)).unwrap().discount_percent, 10);
        assert_eq!(vm.selected().unwrap().ticket_id, TicketId(1));
        assert_eq!(http.requests().len(), 3);
        assert!(!vm.is_busy());
        assert_eq!(vm.error(), None);
    }

    #[test]
    fn commands_follow_ticket_status() {
        let (vm, _) = loaded();
        assert!(vm.can_close());
        assert!(!vm.can_open());

        vm.select_ticket(TicketId(2));
        assert!(!vm.can_close());
        assert!(vm.can_open());
    }

    #[test]
    fn nothing_is_executable_without_selection_or_while_busy() {
        let vm = TicketViewModel::new(TicketApiService::new(MockHttpClient::default()));
        assert!(!vm.can_open() && !vm.can_close() && !vm.can_assign());

        let (vm, _) = loaded();
        vm.select_user(Some(UserId(1)));
        vm.busy.set(true);
        assert!(!vm.can_open() && !vm.can_close() && !vm.can_assign());
    }

    #[test]
    fn assign_requires_an_agent() {
        let (vm, _) = loaded();
        assert!(!vm.can_assign());
        assert_eq!(vm.assign_hint(), None);

        vm.select_user(Some(UserId(2)));
        assert!(!vm.can_assign());
        assert_eq!(
            vm.assign_hint().as_deref(),
            Some("Bob has role Customer, only agents can be assigned")
        );

        vm.select_user(Some(UserId(1)));
        assert!(vm.can_assign());
        assert_eq!(vm.assign_hint(), None);
    }

    #[test]
    fn close_syncs_ticket_and_discount() {
        let (vm, http) = loaded();
        http.respond_json(200, &ticket_dto(1, TicketStatus::Closed))
            .respond_json(200, &tenant_dto(5, 0));

        block_on(vm.close_ticket());

        assert_eq!(status(&vm, 1), TicketStatus::Closed);
        assert_eq!(vm.tenant(TenantId(1)).unwrap().discount_percent, 0);
        assert_eq!(
            http.requests()[3..],
            [
                HttpRequest::post("/api/tickets/1/close", None),
                HttpRequest::post("/api/tenants/1/discount", None),
            ]
        );
        assert!(vm.can_open());
        assert!(!vm.is_busy());
    }

    #[test]
    fn open_syncs_ticket() {
        let (vm, http) = loaded();
        vm.select_ticket(TicketId(2));
        http.respond_json(200, &ticket_dto(2, TicketStatus::Open))
            .respond_json(200, &tenant_dto(7, 10));

        block_on(vm.open_ticket());

        assert_eq!(status(&vm, 2), TicketStatus::Open);
        assert_eq!(vm.tenant(TenantId(1)).unwrap().open_tickets, 7);
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
        vm.select_user(Some(UserId(2)));
        block_on(vm.assign_agent());
        assert_eq!(http.requests().len(), 3);
    }

    #[test]
    fn assign_updates_ticket() {
        let (vm, http) = loaded();
        vm.select_user(Some(UserId(1)));
        http.respond_json(
            200,
            &TicketDto {
                assigned_agent: Some(1),
                ..ticket_dto(1, TicketStatus::InProgress)
            },
        );

        block_on(vm.assign_agent());

        let ticket = vm.selected().unwrap();
        assert_eq!(ticket.status, TicketStatus::InProgress);
        assert_eq!(ticket.assigned_agent, Some(UserId(1)));
        assert!(!vm.can_assign());
    }
}
