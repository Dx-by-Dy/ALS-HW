use std::convert::Infallible;

use ticketing::application::notifications::{Notification, Notifier};

/// Доставка уведомлений в журнал сервера. Настоящая почта или мессенджер
/// подключились бы как другая реализация порта [`Notifier`].
#[derive(Debug, Clone, Copy, Default)]
pub struct LogNotifier;

impl Notifier for LogNotifier {
    type Error = Infallible;

    async fn send(&self, notification: &Notification) -> Result<(), Infallible> {
        println!("notification {notification}");
        Ok(())
    }
}
