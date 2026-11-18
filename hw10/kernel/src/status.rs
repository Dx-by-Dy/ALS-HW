use std::error::Error;
use std::fmt;

/// Статус тикета. Его понимают обе системы: тикеты меняют статус, CRM показывает его клиенту.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

    /// Допустим ли переход из `self` в `next`.
    ///
    /// - `Open → InProgress` — назначили агента;
    /// - `InProgress → InProgress` — тикет передали другому агенту;
    /// - `Open | InProgress → Closed` — тикет закрыли;
    /// - `Closed → Open` — тикет открыли заново, и ему снова нужен агент.
    ///
    /// Всё остальное запрещено: закрытый тикет не берут в работу, не закрывают повторно,
    /// а открытый не открывают ещё раз.
    pub fn can_become(self, next: TicketStatus) -> bool {
        use TicketStatus::*;
        matches!(
            (self, next),
            (Open, InProgress)
                | (InProgress, InProgress)
                | (Open | InProgress, Closed)
                | (Closed, Open)
        )
    }

    /// Переход в `next`, если он допустим.
    pub fn transition_to(self, next: TicketStatus) -> Result<TicketStatus, InvalidTransition> {
        if self.can_become(next) {
            Ok(next)
        } else {
            Err(InvalidTransition {
                from: self,
                to: next,
            })
        }
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

/// Переход между статусами, который запрещают общие правила.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTransition {
    pub from: TicketStatus,
    pub to: TicketStatus,
}

impl fmt::Display for InvalidTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ticket status cannot change from {} to {}",
            self.from, self.to
        )
    }
}

impl Error for InvalidTransition {}

#[cfg(test)]
mod tests {
    use super::TicketStatus::{self, *};
    use super::*;

    const ALL: [TicketStatus; 3] = [Open, InProgress, Closed];

    #[test]
    fn allowed_transitions_are_exactly_the_lifecycle_ones() {
        let allowed: Vec<_> = ALL
            .iter()
            .flat_map(|&from| ALL.iter().map(move |&to| (from, to)))
            .filter(|&(from, to)| from.can_become(to))
            .collect();
        assert_eq!(
            allowed,
            [
                (Open, InProgress),
                (Open, Closed),
                (InProgress, InProgress),
                (InProgress, Closed),
                (Closed, Open),
            ]
        );
    }

    #[test]
    fn allowed_transition_returns_new_status() {
        assert_eq!(Open.transition_to(InProgress), Ok(InProgress));
        assert_eq!(Closed.transition_to(Open), Ok(Open));
    }

    #[test]
    fn forbidden_transition_names_both_statuses() {
        let error = Closed.transition_to(InProgress).unwrap_err();
        assert_eq!(
            error,
            InvalidTransition {
                from: Closed,
                to: InProgress
            }
        );
        assert_eq!(
            error.to_string(),
            "ticket status cannot change from Closed to InProgress"
        );
    }

    #[test]
    fn closed_ticket_cannot_be_closed_again_and_open_cannot_be_reopened() {
        assert!(!Closed.can_become(Closed));
        assert!(!Open.can_become(Open));
        assert!(!InProgress.can_become(Open));
    }

    #[test]
    fn only_closed_ticket_is_not_open() {
        assert!(Open.is_open());
        assert!(InProgress.is_open());
        assert!(!Closed.is_open());
    }
}
