use super::aggregate::Aggregate;

/// Эволюция состояния: следующее состояние по текущему и произошедшему событию.
///
/// Событие — уже свершившийся факт, поэтому `apply_event` не может его отклонить.
/// Невозможное событие означает испорченный стрим, и реализация вправе паниковать.
pub trait EventHandler: Aggregate {
    fn apply_event(self, event: &Self::Event) -> Self;
}
