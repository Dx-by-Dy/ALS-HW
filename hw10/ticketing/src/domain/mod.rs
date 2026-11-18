mod event;

pub mod partner;
pub mod tenant;
pub mod ticket;
pub mod user;

/// Время и SLA — часть Shared Kernel: их так же понимает CRM.
pub use kernel::{sla, time};

pub use event::DomainEvent;
