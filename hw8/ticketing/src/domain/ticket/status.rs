use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TicketStatus {
    /// Создан, агент не назначен.
    Open,
    /// Назначен агент.
    InProgress,
    Closed,
}

impl TicketStatus {
    pub fn is_open(self) -> bool {
        !matches!(self, Self::Closed)
    }
}

impl fmt::Display for TicketStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Open => "Open",
            Self::InProgress => "InProgress",
            Self::Closed => "Closed",
        };
        f.write_str(name)
    }
}
