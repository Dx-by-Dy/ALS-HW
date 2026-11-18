//! Минимальный HTTP-клиент, за которым прячется браузерный `fetch`: в тестах его заменяет заглушка.

use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: Method,
    pub path: String,
    /// JSON-тело запроса.
    pub body: Option<String>,
    /// Значение заголовка `Authorization`.
    pub authorization: Option<String>,
}

impl HttpRequest {
    pub fn get(path: impl Into<String>) -> Self {
        Self {
            method: Method::Get,
            path: path.into(),
            body: None,
            authorization: None,
        }
    }

    pub fn post(path: impl Into<String>, body: Option<String>) -> Self {
        Self {
            method: Method::Post,
            path: path.into(),
            body,
            authorization: None,
        }
    }

    pub fn with_authorization(self, value: impl Into<String>) -> Self {
        Self {
            authorization: Some(value.into()),
            ..self
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

impl HttpResponse {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Запрос не дошёл до сервера или ответ не удалось прочитать.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportError(pub String);

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for TransportError {}

pub trait HttpClient {
    /// Любой ответ сервера, в том числе с кодом ошибки, — это `Ok`.
    fn send(
        &self,
        request: HttpRequest,
    ) -> impl Future<Output = Result<HttpResponse, TransportError>>;
}

/// `fetch` браузера. Запросы идут на тот же origin, с которого загружен клиент.
#[derive(Debug, Clone, Copy, Default)]
pub struct BrowserHttpClient;

impl HttpClient for BrowserHttpClient {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let mut builder = match request.method {
            Method::Get => gloo_net::http::Request::get(&request.path),
            Method::Post => gloo_net::http::Request::post(&request.path),
        };
        if let Some(authorization) = &request.authorization {
            builder = builder.header("Authorization", authorization);
        }
        let sent = match request.body {
            Some(body) => {
                builder
                    .header("Content-Type", "application/json")
                    .body(body)
                    .map_err(transport)?
                    .send()
                    .await
            }
            None => builder.send().await,
        };
        let response = sent.map_err(transport)?;
        Ok(HttpResponse {
            status: response.status(),
            body: response.text().await.map_err(transport)?,
        })
    }
}

fn transport(e: gloo_net::Error) -> TransportError {
    TransportError(e.to_string())
}
