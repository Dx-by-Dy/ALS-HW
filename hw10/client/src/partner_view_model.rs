//! ViewModel панели партнёра: вход по токену, тикеты партнёра и его отчёты.
//!
//! Панель изображает приложение партнёра: она знает только токен и опубликованный язык
//! Open-Host Service. Своей подписки на изменения у OHS нет, поэтому список обновляется
//! после каждого действия и по [`refresh`](PartnerViewModel::refresh), который представление
//! вызывает, когда основной клиент узнаёт о новых изменениях на сервере.

use std::sync::Arc;

use leptos::prelude::*;
use partner_api::{PartnerTicket, PartnerTicketStatus, UpdateStatus};

use crate::model::http::HttpClient;
use crate::model::partner_portal::PartnerPortalService;
use crate::model::ticket_api::ApiError;

/// HTTP-статус «токен не принят».
const UNAUTHORIZED: u16 = 401;

pub struct PartnerViewModel<H: 'static> {
    /// Поле ввода токена.
    pub token_input: RwSignal<String>,
    /// Поле ввода комментария к выбранному тикету.
    pub comment: RwSignal<String>,
    portal: StoredValue<Arc<PartnerPortalService<H>>>,
    /// Токен, с которым выполнен вход.
    token: RwSignal<Option<String>>,
    tickets: RwSignal<Vec<PartnerTicket>>,
    selected: RwSignal<Option<usize>>,
    busy: RwSignal<bool>,
    error: RwSignal<Option<String>>,
}

impl<H> Clone for PartnerViewModel<H> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<H> Copy for PartnerViewModel<H> {}

impl<H: HttpClient + Send + Sync + 'static> PartnerViewModel<H> {
    pub fn new(portal: PartnerPortalService<H>) -> Self {
        Self {
            token_input: RwSignal::new(String::new()),
            comment: RwSignal::new(String::new()),
            portal: StoredValue::new(Arc::new(portal)),
            token: RwSignal::new(None),
            tickets: RwSignal::new(Vec::new()),
            selected: RwSignal::new(None),
            busy: RwSignal::new(false),
            error: RwSignal::new(None),
        }
    }

    // --- Свойства для привязки ---

    pub fn is_signed_in(&self) -> bool {
        self.token.with(Option::is_some)
    }

    pub fn tickets(&self) -> Vec<PartnerTicket> {
        self.tickets.get()
    }

    pub fn selected(&self) -> Option<PartnerTicket> {
        let id = self.selected.get()?;
        self.tickets
            .with(|tickets| tickets.iter().find(|t| t.ticket_id == id).cloned())
    }

    pub fn is_selected(&self, ticket_id: usize) -> bool {
        self.selected.get() == Some(ticket_id)
    }

    pub fn is_busy(&self) -> bool {
        self.busy.get()
    }

    pub fn error(&self) -> Option<String> {
        self.error.get()
    }

    pub fn select(&self, ticket_id: usize) {
        self.selected.set(Some(ticket_id));
        self.error.set(None);
    }

    // --- Доступность команд ---

    pub fn can_sign_in(&self) -> bool {
        !self.busy.get() && self.token_input.with(|t| !t.trim().is_empty())
    }

    pub fn can_resolve(&self) -> bool {
        self.is_signed_in()
            && !self.busy.get()
            && self
                .selected()
                .is_some_and(|t| t.status == PartnerTicketStatus::Open)
    }

    pub fn can_report(&self) -> bool {
        self.can_resolve() && self.comment.with(|c| !c.trim().is_empty())
    }

    // --- Команды ---

    /// Входит с токеном из поля ввода: токен принят, если сервис отдал список тикетов.
    pub async fn sign_in(&self) {
        if !untrack(|| self.can_sign_in()) {
            return;
        }
        let token = self.token_input.get_untracked().trim().to_owned();
        self.busy.set(true);
        self.error.set(None);
        let result = self.portal.get_value().tickets(&token).await;
        self.busy.set(false);
        match result {
            Ok(tickets) => {
                self.token.set(Some(token));
                self.show(tickets);
            }
            Err(e) => self.error.set(Some(e.to_string())),
        }
    }

    pub fn sign_out(&self) {
        self.token.set(None);
        self.tickets.set(Vec::new());
        self.selected.set(None);
        self.comment.set(String::new());
        self.error.set(None);
    }

    /// Перечитывает тикеты партнёра. Если токен больше не принимают, выходит.
    pub async fn refresh(&self) {
        let Some(token) = self.token.get_untracked() else {
            return;
        };
        let result = self.portal.get_value().tickets(&token).await;
        // Пока ответ шёл, могли выйти или войти другим партнёром.
        if self.token.get_untracked().as_deref() != Some(token.as_str()) {
            return;
        }
        match result {
            Ok(tickets) => self.show(tickets),
            Err(ApiError::Server {
                status: UNAUTHORIZED,
                message,
            }) => {
                self.sign_out();
                self.error.set(Some(message));
            }
            Err(e) => self.error.set(Some(e.to_string())),
        }
    }

    pub async fn report_progress(&self) {
        if untrack(|| self.can_report()) {
            self.update(UpdateStatus::InProgress).await;
        }
    }

    pub async fn resolve(&self) {
        if untrack(|| self.can_resolve()) {
            self.update(UpdateStatus::Resolved).await;
        }
    }

    async fn update(&self, status: UpdateStatus) {
        let (Some(token), Some(ticket_id)) =
            (self.token.get_untracked(), self.selected.get_untracked())
        else {
            return;
        };
        let comment = self.comment.get_untracked();
        self.busy.set(true);
        self.error.set(None);
        let result = self
            .portal
            .get_value()
            .update(&token, ticket_id, status, &comment)
            .await;
        self.busy.set(false);
        match result {
            Ok(ticket) => {
                self.tickets.update(|tickets| {
                    if let Some(old) = tickets.iter_mut().find(|t| t.ticket_id == ticket_id) {
                        *old = ticket;
                    }
                });
                self.comment.set(String::new());
            }
            Err(e) => self.error.set(Some(e.to_string())),
        }
    }

    /// Показывает список; выбранный тикет остаётся, если он всё ещё в списке.
    fn show(&self, tickets: Vec<PartnerTicket>) {
        let selected = self.selected.get_untracked();
        let keep = selected.filter(|id| tickets.iter().any(|t| t.ticket_id == *id));
        self.selected
            .set(keep.or_else(|| tickets.first().map(|t| t.ticket_id)));
        self.tickets.set(tickets);
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use partner_api::{ErrorCode, PartnerError, PartnerUpdate};

    use super::*;
    use crate::model::mock_http::MockHttpClient;
    use crate::model::partner_portal::fixtures::partner_ticket;

    fn signed_in() -> (PartnerViewModel<MockHttpClient>, MockHttpClient) {
        let http = MockHttpClient::default();
        let vm = PartnerViewModel::new(PartnerPortalService::new(http.clone()));
        vm.token_input.set(" fixit-token ".to_owned());
        http.respond_json(
            200,
            &vec![
                partner_ticket(4, PartnerTicketStatus::Open),
                partner_ticket(5, PartnerTicketStatus::Resolved),
            ],
        );
        block_on(vm.sign_in());
        (vm, http)
    }

    fn unauthorized() -> PartnerError {
        PartnerError {
            code: ErrorCode::Unauthorized,
            message: "a valid partner token is required".to_owned(),
        }
    }

    #[test]
    fn sign_in_shows_partner_tickets_and_selects_first() {
        let (vm, http) = signed_in();
        assert!(vm.is_signed_in());
        assert_eq!(vm.tickets().len(), 2);
        assert_eq!(vm.selected().unwrap().ticket_id, 4);
        assert_eq!(
            http.requests()[0].authorization.as_deref(),
            Some("Bearer fixit-token")
        );
    }

    #[test]
    fn rejected_token_does_not_sign_in() {
        let http = MockHttpClient::default();
        let vm = PartnerViewModel::new(PartnerPortalService::new(http.clone()));
        assert!(!vm.can_sign_in());
        vm.token_input.set("stolen".to_owned());
        http.respond_json(401, &unauthorized());
        block_on(vm.sign_in());
        assert!(!vm.is_signed_in());
        assert_eq!(
            vm.error().as_deref(),
            Some("server rejected the request (401): a valid partner token is required")
        );
    }

    #[test]
    fn progress_needs_a_comment_and_open_ticket() {
        let (vm, _) = signed_in();
        assert!(vm.can_resolve());
        assert!(!vm.can_report());
        vm.comment.set("Part ordered".to_owned());
        assert!(vm.can_report());
        vm.select(5);
        assert!(!vm.can_report());
        assert!(!vm.can_resolve());
    }

    #[test]
    fn report_replaces_ticket_with_answer_and_clears_comment() {
        let (vm, http) = signed_in();
        vm.comment.set("Part ordered".to_owned());
        let mut answer = partner_ticket(4, PartnerTicketStatus::Open);
        answer.updates.push(PartnerUpdate {
            comment: "Part ordered".to_owned(),
            at: 10,
        });
        http.respond_json(200, &answer);
        block_on(vm.report_progress());
        assert_eq!(vm.selected().unwrap().updates.len(), 1);
        assert_eq!(vm.comment.get(), "");
        let request = http.requests().pop().unwrap();
        assert_eq!(request.path, "/partner/v1/tickets/4/updates");
        assert_eq!(
            request.body.as_deref(),
            Some(r#"{"status":"in_progress","comment":"Part ordered"}"#)
        );
    }

    #[test]
    fn failed_resolve_keeps_comment_and_shows_error() {
        let (vm, http) = signed_in();
        vm.comment.set("Fixed".to_owned());
        http.respond_json(
            409,
            &PartnerError {
                code: ErrorCode::Closed,
                message: "ticket-4 is closed".to_owned(),
            },
        );
        block_on(vm.resolve());
        assert_eq!(vm.comment.get(), "Fixed");
        assert_eq!(
            vm.error().as_deref(),
            Some("server rejected the request (409): ticket-4 is closed")
        );
    }

    #[test]
    fn refresh_keeps_selection_and_signs_out_when_token_is_revoked() {
        let (vm, http) = signed_in();
        vm.select(5);
        http.respond_json(200, &vec![partner_ticket(5, PartnerTicketStatus::Resolved)]);
        block_on(vm.refresh());
        assert_eq!(vm.selected().unwrap().ticket_id, 5);

        http.respond_json(401, &unauthorized());
        block_on(vm.refresh());
        assert!(!vm.is_signed_in());
        assert!(vm.tickets().is_empty());
        assert_eq!(
            vm.error().as_deref(),
            Some("a valid partner token is required")
        );
    }
}
