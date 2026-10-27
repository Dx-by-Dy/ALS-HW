use std::error::Error;
use std::fmt;

use ticketing::application::error::ServiceError;
use ticketing::core::envelope::{Envelope, Version};
use ticketing::domain::DomainEvent;
use tokio::sync::{mpsc, oneshot};

use crate::command::Command;

pub(crate) type CommandReply = oneshot::Sender<Result<Committed, ServiceError>>;
pub(crate) type EventReceiver = mpsc::UnboundedReceiver<Envelope<DomainEvent>>;

/// Сообщения актору.
pub(crate) enum Message {
    Execute {
        command: Command,
        reply: CommandReply,
    },
    Subscribe {
        after: Version,
        reply: oneshot::Sender<EventReceiver>,
    },
}

/// Результат выполненной команды.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Committed {
    /// Версия после команды. Если команда ничего не записала
    /// (например, закрывать по неактивности было нечего), — текущая версия.
    pub version: Version,
    /// Записанные события в порядке записи.
    pub events: Vec<Envelope<DomainEvent>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// Команду отклонил сервис или агрегат.
    Rejected(ServiceError),
    /// Актор не работает: команда не принята или ответ на неё потерян.
    Unavailable,
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(e) => e.fmt(f),
            Self::Unavailable => f.write_str("command handler is not running"),
        }
    }
}

impl Error for CommandError {}

impl From<ServiceError> for CommandError {
    fn from(e: ServiceError) -> Self {
        Self::Rejected(e)
    }
}

/// Ручка для общения с актором. Клонируется дёшево; актор живёт, пока жив хотя бы один клон.
#[derive(Debug, Clone)]
pub struct CommandBus {
    sender: mpsc::Sender<Message>,
}

impl CommandBus {
    pub(crate) fn new(sender: mpsc::Sender<Message>) -> Self {
        Self { sender }
    }

    /// Ставит команду в очередь актора и ждёт результата.
    pub async fn send(&self, command: impl Into<Command>) -> Result<Committed, CommandError> {
        let (reply, response) = oneshot::channel();
        let command = command.into();
        self.request(Message::Execute { command, reply }).await?;
        let result = response.await.map_err(|_| CommandError::Unavailable)?;
        Ok(result?)
    }

    /// Поток событий с версиями больше `after`: сначала уже записанные, затем новые
    /// по мере записи, без пропусков и повторов.
    pub async fn subscribe(&self, after: Version) -> Result<EventReceiver, CommandError> {
        let (reply, response) = oneshot::channel();
        self.request(Message::Subscribe { after, reply }).await?;
        response.await.map_err(|_| CommandError::Unavailable)
    }

    async fn request(&self, message: Message) -> Result<(), CommandError> {
        self.sender
            .send(message)
            .await
            .map_err(|_| CommandError::Unavailable)
    }
}
