use std::collections::BTreeMap;

use super::{Saga, SagaEffects, deadlines::DeadlineQueue};

/// Ведёт экземпляры саги и их дедлайны: находит экземпляр по событию, применяет [`Effects`]
/// и отдаёт команды, которые нужно отправить. Сам ничего не отправляет и времени не знает.
///
/// [`Effects`]: super::effects::Effects
pub struct SagaManager<S: Saga> {
    instances: BTreeMap<S::Id, S>,
    deadlines: DeadlineQueue<S::Id, S::Deadline, S::Time>,
}

impl<S: Saga> Default for SagaManager<S> {
    fn default() -> Self {
        Self {
            instances: BTreeMap::new(),
            deadlines: DeadlineQueue::default(),
        }
    }
}

impl<S: Saga> SagaManager<S> {
    /// Передаёт событие экземпляру. Если экземпляра нет, событие может его запустить.
    pub fn on_event(&mut self, event: &S::Event) -> Vec<S::Command> {
        let Some(id) = S::id_of(event) else {
            return Vec::new();
        };
        let effects = if let Some(saga) = self.instances.get_mut(&id) {
            saga.on_event(event)
        } else if let Some((saga, effects)) = S::start(event) {
            self.instances.insert(id.clone(), saga);
            effects
        } else {
            return Vec::new();
        };
        self.apply(id, effects)
    }

    /// Срабатывают все дедлайны со сроком не позже `now`, от ранних к поздним. Дедлайн, который
    /// сага назначила по ходу и срок которого уже наступил, сработает здесь же.
    pub fn fire_due(&mut self, now: &S::Time) -> Vec<S::Command> {
        let mut commands = Vec::new();
        while let Some((id, deadline)) = self.deadlines.pop_due(now) {
            // Дедлайны живут не дольше экземпляра: их удаляют вместе с ним.
            let Some(saga) = self.instances.get_mut(&id) else {
                continue;
            };
            let effects = saga.on_deadline(deadline);
            commands.extend(self.apply(id, effects));
        }
        commands
    }

    /// Ближайший срок, к которому что-то сработает.
    pub fn next_deadline(&self) -> Option<&S::Time> {
        self.deadlines.next_at()
    }

    pub fn is_running(&self, id: &S::Id) -> bool {
        self.instances.contains_key(id)
    }

    /// Назначенные дедлайны экземпляра.
    pub fn deadlines_of(&self, id: &S::Id) -> Vec<(S::Deadline, S::Time)> {
        self.deadlines.of(id)
    }

    fn apply(&mut self, id: S::Id, effects: SagaEffects<S>) -> Vec<S::Command> {
        if effects.finished {
            self.instances.remove(&id);
            self.deadlines.cancel_all(&id);
        } else {
            for (deadline, at) in effects.deadlines {
                self.deadlines.schedule(id.clone(), deadline, at);
            }
        }
        effects.commands
    }
}

#[cfg(test)]
mod tests {
    use super::super::effects::Effects;
    use super::*;

    #[derive(Debug, Clone, Copy)]
    enum Event {
        Placed(u32, u64),
        /// Активность: сроки считаются заново.
        Touched(u32, u64),
        /// Отмена назначается на указанный момент.
        Hurried(u32, u64),
        Paid(u32),
        Unrelated,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Alarm {
        Remind,
        Nudge,
        Cancel,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Cmd {
        Confirm(u32),
        Remind(u32),
        Nudge(u32),
        Cancel(u32),
    }

    /// Заказ: подтвердить сразу, напомнить через 3, ещё раз через 1 после напоминания,
    /// отменить через 10 от последней активности, завершить при оплате или отмене.
    struct Order {
        id: u32,
        active_at: u64,
    }

    fn timers(at: u64) -> Effects<Cmd, Alarm, u64> {
        Effects::none()
            .schedule(Alarm::Remind, at + 3)
            .schedule(Alarm::Cancel, at + 10)
    }

    impl Saga for Order {
        type Event = Event;
        type Command = Cmd;
        type Id = u32;
        type Deadline = Alarm;
        type Time = u64;

        fn id_of(event: &Event) -> Option<u32> {
            match *event {
                Event::Placed(id, _)
                | Event::Touched(id, _)
                | Event::Hurried(id, _)
                | Event::Paid(id) => Some(id),
                Event::Unrelated => None,
            }
        }

        fn start(event: &Event) -> Option<(Self, SagaEffects<Self>)> {
            let Event::Placed(id, at) = *event else {
                return None;
            };
            let order = Order { id, active_at: at };
            Some((order, timers(at).command(Cmd::Confirm(id))))
        }

        fn on_event(&mut self, event: &Event) -> SagaEffects<Self> {
            match *event {
                Event::Touched(_, at) => {
                    self.active_at = at;
                    timers(at)
                }
                Event::Hurried(_, at) => Effects::none().schedule(Alarm::Cancel, at),
                Event::Paid(_) => Effects::none().finish(),
                Event::Placed(..) | Event::Unrelated => Effects::none(),
            }
        }

        fn on_deadline(&mut self, deadline: Alarm) -> SagaEffects<Self> {
            match deadline {
                Alarm::Remind => Effects::none()
                    .command(Cmd::Remind(self.id))
                    .schedule(Alarm::Nudge, self.active_at + 4),
                Alarm::Nudge => Effects::none().command(Cmd::Nudge(self.id)),
                Alarm::Cancel => Effects::none().command(Cmd::Cancel(self.id)).finish(),
            }
        }
    }

    fn manager_with(events: &[Event]) -> SagaManager<Order> {
        let mut manager = SagaManager::default();
        for event in events {
            manager.on_event(event);
        }
        manager
    }

    #[test]
    fn starting_event_runs_instance_and_schedules_deadlines() {
        let mut manager = SagaManager::<Order>::default();
        assert_eq!(manager.on_event(&Event::Placed(1, 0)), [Cmd::Confirm(1)]);
        assert!(manager.is_running(&1));
        assert_eq!(
            manager.deadlines_of(&1),
            [(Alarm::Remind, 3), (Alarm::Cancel, 10)]
        );
        assert_eq!(manager.next_deadline(), Some(&3));
    }

    #[test]
    fn other_events_do_not_start_an_instance() {
        let mut manager = SagaManager::<Order>::default();
        assert_eq!(manager.on_event(&Event::Touched(1, 0)), []);
        assert_eq!(manager.on_event(&Event::Paid(1)), []);
        assert_eq!(manager.on_event(&Event::Unrelated), []);
        assert!(!manager.is_running(&1));
        assert_eq!(manager.next_deadline(), None);
    }

    #[test]
    fn starting_event_does_not_restart_running_instance() {
        let mut manager = manager_with(&[Event::Placed(1, 0)]);
        assert_eq!(manager.on_event(&Event::Placed(1, 50)), []);
        assert_eq!(
            manager.deadlines_of(&1),
            [(Alarm::Remind, 3), (Alarm::Cancel, 10)]
        );
    }

    #[test]
    fn nothing_fires_before_the_deadline() {
        let mut manager = manager_with(&[Event::Placed(1, 0)]);
        assert_eq!(manager.fire_due(&2), []);
        assert_eq!(manager.deadlines_of(&1).len(), 2);
    }

    #[test]
    fn deadline_fires_at_its_time_once_and_keeps_the_others() {
        let mut manager = manager_with(&[Event::Placed(1, 0)]);
        assert_eq!(manager.fire_due(&3), [Cmd::Remind(1)]);
        assert_eq!(manager.fire_due(&3), []);
        assert!(manager.is_running(&1));
        assert_eq!(
            manager.deadlines_of(&1),
            [(Alarm::Nudge, 4), (Alarm::Cancel, 10)]
        );
    }

    #[test]
    fn deadline_can_schedule_the_next_one() {
        let mut manager = manager_with(&[Event::Placed(1, 0)]);
        manager.fire_due(&3);
        assert_eq!(manager.fire_due(&4), [Cmd::Nudge(1)]);
    }

    #[test]
    fn finishing_removes_instance_and_all_its_deadlines() {
        let mut manager = manager_with(&[Event::Placed(1, 0), Event::Placed(2, 0)]);
        assert_eq!(manager.on_event(&Event::Paid(1)), []);
        assert!(!manager.is_running(&1));
        assert_eq!(manager.deadlines_of(&1), []);

        // Второй экземпляр не задет, а у первого больше ничего не сработает.
        assert_eq!(manager.fire_due(&3), [Cmd::Remind(2)]);
    }

    #[test]
    fn activity_replaces_deadlines_so_old_ones_do_not_fire() {
        let mut manager = manager_with(&[Event::Placed(1, 0), Event::Touched(1, 5)]);
        assert_eq!(
            manager.deadlines_of(&1),
            [(Alarm::Remind, 8), (Alarm::Cancel, 15)]
        );
        assert_eq!(manager.fire_due(&7), []);
        assert_eq!(manager.fire_due(&8), [Cmd::Remind(1)]);
    }

    #[test]
    fn overdue_deadlines_fire_in_order_in_one_pass() {
        let mut manager = manager_with(&[Event::Placed(1, 0)]);
        assert_eq!(
            manager.fire_due(&100),
            [Cmd::Remind(1), Cmd::Nudge(1), Cmd::Cancel(1)]
        );
        assert!(!manager.is_running(&1));
        assert_eq!(manager.next_deadline(), None);
    }

    #[test]
    fn finishing_in_a_pass_drops_its_remaining_due_deadlines() {
        let mut manager = manager_with(&[Event::Placed(1, 0), Event::Hurried(1, 1)]);
        assert_eq!(manager.fire_due(&100), [Cmd::Cancel(1)]);
        assert!(!manager.is_running(&1));
    }

    #[test]
    fn finished_process_can_start_again() {
        let mut manager = manager_with(&[Event::Placed(1, 0), Event::Paid(1)]);
        assert_eq!(manager.on_event(&Event::Placed(1, 20)), [Cmd::Confirm(1)]);
        assert_eq!(
            manager.deadlines_of(&1),
            [(Alarm::Remind, 23), (Alarm::Cancel, 30)]
        );
    }
}
