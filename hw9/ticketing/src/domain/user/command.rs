use super::{Role, UserId};

/// Команды агрегата `User`. Идентификатор выдаёт сервис приложения до вызова `decide`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserCommand {
    Register {
        user_id: UserId,
        user_name: String,
        role: Role,
    },
}
