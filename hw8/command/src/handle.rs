use std::error::Error;
use std::fmt;

use framework::cqrs::{
    command_bus::CommandBus,
    envelope::{Committed, Envelope, Version},
    event_source::{EventSource, Subscription},
};
use ticketing::application::error::ServiceError;
use ticketing::domain::DomainEvent;
use tokio::sync::{mpsc, oneshot};

use crate::command::Command;

pub(crate) type CommandReply = oneshot::Sender<Result<Committed<DomainEvent>, ServiceError>>;

/// Сообщения актору.
pub(crate) enum Message {
    Execute {
        command: Command,
        reply: CommandReply,
    },
    Subscribe {
        after: Version,
        reply: oneshot::Sender<EventSubscription>,
    },
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
pub struct CommandActorHandle {
    sender: mpsc::Sender<Message>,
}

impl CommandActorHandle {
    pub(crate) fn new(sender: mpsc::Sender<Message>) -> Self {
        Self { sender }
    }

    async fn request(&self, message: Message) -> Result<(), CommandError> {
        self.sender
            .send(message)
            .await
            .map_err(|_| CommandError::Unavailable)
    }
}

/// Принимает любую команду, которая превращается в [`Command`], и ставит её в очередь актора.
impl<C: Into<Command> + Send> CommandBus<C> for CommandActorHandle {
    type Event = DomainEvent;
    type Error = CommandError;

    async fn send(&self, command: C) -> Result<Committed<DomainEvent>, CommandError> {
        let (reply, response) = oneshot::channel();
        let command = command.into();
        self.request(Message::Execute { command, reply }).await?;
        let result = response.await.map_err(|_| CommandError::Unavailable)?;
        Ok(result?)
    }
}

impl EventSource<DomainEvent> for CommandActorHandle {
    type Error = CommandError;
    type Subscription = EventSubscription;

    async fn subscribe(&self, after: Version) -> Result<EventSubscription, CommandError> {
        let (reply, response) = oneshot::channel();
        self.request(Message::Subscribe { after, reply }).await?;
        response.await.map_err(|_| CommandError::Unavailable)
    }
}

/// Подписка на журнал актора. Обёртка нужна, чтобы реализовать трейт `framework`
/// для канала tokio: правило сирот не позволяет сделать это напрямую.
#[derive(Debug)]
pub struct EventSubscription(pub(crate) mpsc::UnboundedReceiver<Envelope<DomainEvent>>);

impl EventSubscription {
    /// Уже доступное событие, без ожидания.
    #[cfg(test)]
    pub(crate) fn try_next(&mut self) -> Option<Envelope<DomainEvent>> {
        self.0.try_recv().ok()
    }
}

impl Subscription<DomainEvent> for EventSubscription {
    async fn next(&mut self) -> Option<Envelope<DomainEvent>> {
        self.0.recv().await
    }
}
