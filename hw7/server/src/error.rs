use api::ErrorDto;
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use ticketing::application::error::ServiceError;
use ticketing::domain::{tenant::TenantError, ticket::TicketError, user::UserError};

/// Ошибка, которую обработчик отдаёт клиенту: HTTP-статус и текст для `ErrorDto`.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }
}

impl From<ServiceError> for ApiError {
    fn from(e: ServiceError) -> Self {
        Self {
            status: status_of(&e),
            message: e.to_string(),
        }
    }
}

/// Нет объекта — 404, команда противоречит текущему состоянию — 409,
/// неверные данные в самой команде — 422.
fn status_of(e: &ServiceError) -> StatusCode {
    match e {
        ServiceError::Ticket(TicketError::NotFound)
        | ServiceError::Tenant(TenantError::NotFound)
        | ServiceError::UnknownTenant(_)
        | ServiceError::UnknownUser(_) => StatusCode::NOT_FOUND,
        ServiceError::Ticket(TicketError::NotAnAgent { .. } | TicketError::EmptyDescription)
        | ServiceError::Tenant(TenantError::EmptyName)
        | ServiceError::User(UserError::EmptyName) => StatusCode::UNPROCESSABLE_ENTITY,
        ServiceError::Ticket(_) | ServiceError::Tenant(_) | ServiceError::User(_) => {
            StatusCode::CONFLICT
        }
        ServiceError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorDto {
            error: self.message,
        };
        (self.status, Json(body)).into_response()
    }
}
