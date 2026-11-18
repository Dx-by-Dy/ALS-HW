mod event;

pub mod tenant;
pub mod ticket;
pub mod user;

/// Время — часть Shared Kernel: тот же `Timestamp` понимает CRM.
pub use kernel::time;

pub use event::DomainEvent;
