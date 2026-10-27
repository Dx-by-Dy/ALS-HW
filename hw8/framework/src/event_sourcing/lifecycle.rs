use crate::event_sourcing::{
    aggregate::Aggregate, command_handler::CommandHandler, event_handler::EventHandler,
};

/// Агрегат, которого может ещё не быть: состояние пустого стрима — `None`.
///
/// Домен описывает два случая — «агрегата ещё нет» и «агрегат есть», а реализации
/// [`Aggregate`], [`CommandHandler`] и [`EventHandler`] для `Option<Self>` даёт каркас.
pub trait Lifecycle: Sized {
    type Event;
    type Command;
    type Error;

    /// Решение по команде, пока агрегата нет: обычно создание, остальное — «не найден».
    fn decide_new(command: &Self::Command) -> Result<Self::Event, Self::Error>;

    /// Решение по команде для существующего агрегата.
    fn decide(&self, command: &Self::Command) -> Result<Self::Event, Self::Error>;

    /// Агрегат из первого события стрима.
    fn created(event: &Self::Event) -> Self;

    /// Состояние после очередного события.
    fn evolve(self, event: &Self::Event) -> Self;
}

impl<T: Lifecycle> Aggregate for Option<T> {
    type Event = T::Event;
    type Command = T::Command;
}

impl<T: Lifecycle> CommandHandler for Option<T> {
    type Error = T::Error;

    fn decide(&self, command: &T::Command) -> Result<T::Event, T::Error> {
        match self {
            None => T::decide_new(command),
            Some(aggregate) => aggregate.decide(command),
        }
    }
}

impl<T: Lifecycle> EventHandler for Option<T> {
    fn apply_event(self, event: &T::Event) -> Self {
        Some(match self {
            None => T::created(event),
            Some(aggregate) => aggregate.evolve(event),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_sourcing::event_sourced::EventSourced;

    /// Счётчик, который создаётся с начальным значением и затем только растёт.
    #[derive(Debug, PartialEq)]
    struct Counter(u32);

    #[derive(Debug, PartialEq)]
    enum Command {
        Create(u32),
        Add(u32),
    }

    #[derive(Debug, PartialEq)]
    enum Event {
        Created(u32),
        Added(u32),
    }

    #[derive(Debug, PartialEq)]
    enum Error {
        NotFound,
        AlreadyExists,
    }

    impl Lifecycle for Counter {
        type Event = Event;
        type Command = Command;
        type Error = Error;

        fn decide_new(command: &Command) -> Result<Event, Error> {
            match command {
                Command::Create(n) => Ok(Event::Created(*n)),
                Command::Add(_) => Err(Error::NotFound),
            }
        }

        fn decide(&self, command: &Command) -> Result<Event, Error> {
            match command {
                Command::Create(_) => Err(Error::AlreadyExists),
                Command::Add(n) => Ok(Event::Added(*n)),
            }
        }

        fn created(event: &Event) -> Self {
            match event {
                Event::Created(n) => Counter(*n),
                Event::Added(_) => panic!("added before creation"),
            }
        }

        fn evolve(self, event: &Event) -> Self {
            match event {
                Event::Created(_) => panic!("created twice"),
                Event::Added(n) => Counter(self.0 + n),
            }
        }
    }

    #[test]
    fn missing_aggregate_decides_with_decide_new() {
        assert_eq!(
            None::<Counter>.decide(&Command::Create(1)),
            Ok(Event::Created(1))
        );
        assert_eq!(
            None::<Counter>.decide(&Command::Add(1)),
            Err(Error::NotFound)
        );
    }

    #[test]
    fn existing_aggregate_decides_with_decide() {
        let counter = Some(Counter(1));
        assert_eq!(
            counter.decide(&Command::Create(1)),
            Err(Error::AlreadyExists)
        );
        assert_eq!(counter.decide(&Command::Add(2)), Ok(Event::Added(2)));
    }

    #[test]
    fn first_event_creates_and_next_ones_evolve() {
        let counter = None.recover([Event::Created(1), Event::Added(2), Event::Added(3)]);
        assert_eq!(counter, Some(Counter(6)));
    }

    #[test]
    #[should_panic(expected = "added before creation")]
    fn stream_must_start_with_creation() {
        None::<Counter>.apply_event(&Event::Added(1));
    }
}
