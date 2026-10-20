//! Заглушка HTTP-клиента: отдаёт заранее заданные ответы и запоминает запросы.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde::Serialize;

use super::http::{HttpClient, HttpRequest, HttpResponse, TransportError};

/// Клоны делят одно состояние: один отдаём сервису, через другой задаём ответы и смотрим запросы.
#[derive(Debug, Clone, Default)]
pub struct MockHttpClient(Arc<Mutex<MockState>>);

#[derive(Debug, Default)]
struct MockState {
    responses: VecDeque<Result<HttpResponse, TransportError>>,
    requests: Vec<HttpRequest>,
}

impl MockHttpClient {
    pub fn respond_json(&self, status: u16, body: &impl Serialize) -> &Self {
        self.respond_raw(status, &serde_json::to_string(body).unwrap())
    }

    pub fn respond_error(&self, status: u16, message: &str) -> &Self {
        let body = api::ErrorDto {
            error: message.to_owned(),
        };
        self.respond_json(status, &body)
    }

    pub fn respond_raw(&self, status: u16, body: &str) -> &Self {
        let response = HttpResponse {
            status,
            body: body.to_owned(),
        };
        self.0.lock().unwrap().responses.push_back(Ok(response));
        self
    }

    pub fn fail(&self, message: &str) -> &Self {
        let error = TransportError(message.to_owned());
        self.0.lock().unwrap().responses.push_back(Err(error));
        self
    }

    pub fn requests(&self) -> Vec<HttpRequest> {
        self.0.lock().unwrap().requests.clone()
    }
}

impl HttpClient for MockHttpClient {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let mut state = self.0.lock().unwrap();
        let response = state
            .responses
            .pop_front()
            .unwrap_or_else(|| panic!("unexpected request {request:?}"));
        state.requests.push(request);
        response
    }
}
