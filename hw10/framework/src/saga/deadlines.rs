use std::collections::{BTreeMap, BTreeSet};

/// Дедлайны экземпляров саги: у ключа `K` может быть несколько дедлайнов с разными именами `D`.
///
/// Очередь только хранит сроки. Текущее время ей передают снаружи, поэтому она не знает о часах.
#[derive(Debug, Clone)]
pub struct DeadlineQueue<K, D, T> {
    by_time: BTreeSet<(T, K, D)>,
    by_key: BTreeMap<K, BTreeMap<D, T>>,
}

impl<K, D, T> Default for DeadlineQueue<K, D, T> {
    fn default() -> Self {
        Self {
            by_time: BTreeSet::new(),
            by_key: BTreeMap::new(),
        }
    }
}

impl<K: Ord + Clone, D: Ord + Clone, T: Ord + Clone> DeadlineQueue<K, D, T> {
    /// Назначает дедлайн; прежний дедлайн с тем же именем у этого ключа отменяется.
    pub fn schedule(&mut self, key: K, deadline: D, at: T) {
        let deadlines = self.by_key.entry(key.clone()).or_default();
        if let Some(previous) = deadlines.insert(deadline.clone(), at.clone()) {
            self.by_time
                .remove(&(previous, key.clone(), deadline.clone()));
        }
        self.by_time.insert((at, key, deadline));
    }

    /// Отменяет все дедлайны ключа.
    pub fn cancel_all(&mut self, key: &K) {
        for (deadline, at) in self.by_key.remove(key).into_iter().flatten() {
            self.by_time.remove(&(at, key.clone(), deadline));
        }
    }

    /// Забирает самый ранний дедлайн со сроком не позже `now`. Дедлайн срабатывает один раз.
    pub fn pop_due(&mut self, now: &T) -> Option<(K, D)> {
        if self.next_at()? > now {
            return None;
        }
        let (_, key, deadline) = self.by_time.pop_first()?;
        if let Some(deadlines) = self.by_key.get_mut(&key) {
            deadlines.remove(&deadline);
            if deadlines.is_empty() {
                self.by_key.remove(&key);
            }
        }
        Some((key, deadline))
    }

    /// Срок самого раннего дедлайна.
    pub fn next_at(&self) -> Option<&T> {
        self.by_time.first().map(|(at, _, _)| at)
    }

    /// Дедлайны ключа, упорядоченные по имени.
    pub fn of(&self, key: &K) -> Vec<(D, T)> {
        self.by_key
            .get(key)
            .into_iter()
            .flatten()
            .map(|(deadline, at)| (deadline.clone(), at.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Queue = DeadlineQueue<u32, &'static str, u64>;

    #[test]
    fn empty_queue_has_nothing_due() {
        let mut queue = Queue::default();
        assert_eq!(queue.next_at(), None);
        assert_eq!(queue.pop_due(&u64::MAX), None);
        assert_eq!(queue.of(&1), []);
    }

    #[test]
    fn deadline_is_due_exactly_at_its_time_and_fires_once() {
        let mut queue = Queue::default();
        queue.schedule(1, "remind", 10);
        assert_eq!(queue.pop_due(&9), None);
        assert_eq!(queue.of(&1), [("remind", 10)]);
        assert_eq!(queue.pop_due(&10), Some((1, "remind")));
        assert_eq!(queue.pop_due(&10), None);
        assert_eq!(queue.next_at(), None);
        assert_eq!(queue.of(&1), []);
    }

    #[test]
    fn deadlines_come_out_in_time_order() {
        let mut queue = Queue::default();
        queue.schedule(1, "late", 30);
        queue.schedule(2, "early", 10);
        queue.schedule(1, "middle", 20);
        assert_eq!(queue.next_at(), Some(&10));
        assert_eq!(queue.pop_due(&100), Some((2, "early")));
        assert_eq!(queue.pop_due(&100), Some((1, "middle")));
        assert_eq!(queue.pop_due(&100), Some((1, "late")));
    }

    #[test]
    fn same_name_replaces_previous_deadline() {
        let mut queue = Queue::default();
        queue.schedule(1, "close", 10);
        queue.schedule(1, "close", 50);
        assert_eq!(queue.pop_due(&49), None);
        assert_eq!(queue.of(&1), [("close", 50)]);

        // Перенос на более ранний срок тоже заменяет дедлайн, а не добавляет второй.
        queue.schedule(1, "close", 5);
        assert_eq!(queue.pop_due(&5), Some((1, "close")));
        assert_eq!(queue.pop_due(&100), None);
    }

    #[test]
    fn same_name_of_other_key_is_independent() {
        let mut queue = Queue::default();
        queue.schedule(1, "close", 10);
        queue.schedule(2, "close", 10);
        assert_eq!(queue.pop_due(&10), Some((1, "close")));
        assert_eq!(queue.pop_due(&10), Some((2, "close")));
    }

    #[test]
    fn cancel_all_removes_only_that_key() {
        let mut queue = Queue::default();
        queue.schedule(1, "remind", 10);
        queue.schedule(1, "close", 20);
        queue.schedule(2, "close", 30);
        queue.cancel_all(&1);
        assert_eq!(queue.of(&1), []);
        assert_eq!(queue.next_at(), Some(&30));
        assert_eq!(queue.pop_due(&100), Some((2, "close")));
        assert_eq!(queue.pop_due(&100), None);

        // Отмена того, чего нет, ничего не ломает.
        queue.cancel_all(&99);
    }
}
