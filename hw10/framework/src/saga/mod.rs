//! Саги: долгоживущие процессы, которые следят за событиями и временем и отдают команды.
//!
//! В отличие от [`Policy`](crate::cqrs::policy::Policy), у саги есть состояние и дедлайны.
//! Сама сага — чистый автомат: на событие или сработавший дедлайн она отвечает описанием
//! [`Effects`], а [`SagaManager`](manager::SagaManager) ведёт экземпляры и их дедлайны
//! в [`DeadlineQueue`](deadlines::DeadlineQueue). Модуль не знает ни о часах, ни о задачах:
//! когда спрашивать, что наступило, и куда отправлять команды, решает тот, кто его запускает.
//!
//! Как и в [`Policy`](crate::cqrs::policy::Policy), сага только предлагает команды, а решает
//! агрегат: дедлайн мог устареть, пока команда шла к нему, и отказ — нормальный исход.

pub mod deadlines;
pub mod effects;
pub mod manager;

use effects::Effects;

/// Ответ саги `S` на событие или дедлайн.
pub type SagaEffects<S> = Effects<<S as Saga>::Command, <S as Saga>::Deadline, <S as Saga>::Time>;

/// Описание процесса. Каждому значению [`Id`](Self::Id) соответствует свой экземпляр.
pub trait Saga: Sized {
    type Event;
    type Command;
    /// По чему события находят свой экземпляр.
    type Id: Ord + Clone;
    /// Имя дедлайна внутри экземпляра: повторное назначение заменяет прежнее.
    type Deadline: Ord + Clone;
    type Time: Ord + Clone;

    /// К какому экземпляру относится событие; `None` — событие саге не интересно.
    fn id_of(event: &Self::Event) -> Option<Self::Id>;

    /// Запускает новый экземпляр, если такое событие начинает процесс.
    /// Вызывается, только когда экземпляра с этим идентификатором нет.
    fn start(event: &Self::Event) -> Option<(Self, SagaEffects<Self>)>;

    /// Реакция работающего экземпляра на событие.
    fn on_event(&mut self, event: &Self::Event) -> SagaEffects<Self>;

    /// Реакция работающего экземпляра на наступивший дедлайн.
    fn on_deadline(&mut self, deadline: Self::Deadline) -> SagaEffects<Self>;
}
