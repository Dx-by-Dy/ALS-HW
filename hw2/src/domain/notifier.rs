use std::error::Error;
use std::fmt;

use crate::domain::order::{Order, OrderEvent};

pub trait OrderNotifier {
    fn notify(&self, order: &Order, event: &OrderEvent) -> Result<(), NotifyError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyError {
    reason: String,
}

impl NotifyError {
    pub fn new(reason: impl Into<String>) -> Self {
        Self { reason: reason.into() }
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl fmt::Display for NotifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to notify: {}", self.reason)
    }
}

impl Error for NotifyError {}

#[cfg(test)]
pub mod test_doubles {
    use super::*;
    use std::cell::RefCell;

    #[derive(Debug, Default)]
    pub struct SpyNotifier {
        sent: RefCell<Vec<OrderEvent>>,
        fails: bool,
    }

    impl SpyNotifier {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn failing() -> Self {
            Self { sent: RefCell::new(Vec::new()), fails: true }
        }

        pub fn sent(&self) -> Vec<OrderEvent> {
            self.sent.borrow().clone()
        }
    }

    impl OrderNotifier for SpyNotifier {
        fn notify(&self, _order: &Order, event: &OrderEvent) -> Result<(), NotifyError> {
            self.sent.borrow_mut().push(event.clone());
            if self.fails {
                return Err(NotifyError::new("spy is configured to fail"));
            }
            Ok(())
        }
    }
}
