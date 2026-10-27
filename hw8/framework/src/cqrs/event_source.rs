use super::envelope::{Envelope, Version};

/// Источник событий для читающей стороны: позволяет подписаться на общий поток
/// с любой версии и получить всё, что записано после неё.
pub trait EventSource<E> {
    type Error;
    type Subscription: Subscription<E>;

    /// Поток событий с версиями больше `after`: сначала уже записанные, затем новые
    /// по мере записи, без пропусков и повторов.
    fn subscribe(
        &self,
        after: Version,
    ) -> impl Future<Output = Result<Self::Subscription, Self::Error>> + Send;
}

pub trait Subscription<E> {
    /// Следующее событие; `None` — источник закрыт и событий больше не будет.
    fn next(&mut self) -> impl Future<Output = Option<Envelope<E>>> + Send;
}
