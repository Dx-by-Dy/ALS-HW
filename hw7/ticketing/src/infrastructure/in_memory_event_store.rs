use std::collections::HashMap;

use super::error::StorageError;
use crate::core::event_store::{EventPos, EventStore, StreamId};

/// Стримы событий в памяти процесса: append-only вектор на каждый `StreamId`.
#[derive(Debug)]
pub struct InMemoryEventStore<E> {
    streams: HashMap<StreamId, Vec<E>>,
}

impl<E> Default for InMemoryEventStore<E> {
    fn default() -> Self {
        Self {
            streams: HashMap::new(),
        }
    }
}

impl<E: Clone> EventStore<E> for InMemoryEventStore<E> {
    type Error = StorageError;

    fn event_stream_from_pos(
        &self,
        stream_id: StreamId,
        pos: EventPos,
    ) -> Result<impl IntoIterator<Item = E>, Self::Error> {
        let events = self
            .streams
            .get(&stream_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let tail = events.get(pos.0..).ok_or(StorageError::PosOutOfRange {
            stream_id,
            pos,
            len: events.len(),
        })?;
        Ok(tail.iter().cloned())
    }

    fn store_event(&mut self, stream_id: StreamId, event: E) -> Result<EventPos, Self::Error> {
        let stream = self.streams.entry(stream_id).or_default();
        stream.push(event);
        Ok(EventPos(stream.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(store: &InMemoryEventStore<&'static str>, id: usize, pos: usize) -> Vec<&'static str> {
        store
            .event_stream_from_pos(StreamId(id), EventPos(pos))
            .unwrap()
            .into_iter()
            .collect()
    }

    #[test]
    fn store_returns_stream_length_after_append() {
        let mut store = InMemoryEventStore::default();
        assert_eq!(store.store_event(StreamId(1), "a"), Ok(EventPos(1)));
        assert_eq!(store.store_event(StreamId(1), "b"), Ok(EventPos(2)));
        assert_eq!(store.store_event(StreamId(2), "x"), Ok(EventPos(1)));
    }

    #[test]
    fn stream_is_read_from_position_inclusive() {
        let mut store = InMemoryEventStore::default();
        for event in ["a", "b", "c"] {
            store.store_event(StreamId(1), event).unwrap();
        }
        assert_eq!(read(&store, 1, 0), ["a", "b", "c"]);
        assert_eq!(read(&store, 1, 1), ["b", "c"]);
        assert_eq!(read(&store, 1, 3), Vec::<&str>::new());
    }

    #[test]
    fn missing_stream_is_empty() {
        let store = InMemoryEventStore::default();
        assert_eq!(read(&store, 42, 0), Vec::<&str>::new());
    }

    #[test]
    fn reading_past_the_end_is_an_error() {
        let mut store = InMemoryEventStore::default();
        store.store_event(StreamId(1), "a").unwrap();
        assert!(matches!(
            store.event_stream_from_pos(StreamId(1), EventPos(2)),
            Err(StorageError::PosOutOfRange { len: 1, .. })
        ));
    }
}
