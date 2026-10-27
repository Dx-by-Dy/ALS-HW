use std::collections::HashMap;

use super::error::StorageError;
use crate::core::{
    event_store::{EventPos, StreamId},
    snapshot_store::SnapshotStore,
};

/// Хранит только последний снапшот каждого стрима — более старые для восстановления не нужны.
#[derive(Debug)]
pub struct InMemorySnapshotStore<A> {
    snapshots: HashMap<StreamId, (A, EventPos)>,
}

impl<A> Default for InMemorySnapshotStore<A> {
    fn default() -> Self {
        Self {
            snapshots: HashMap::new(),
        }
    }
}

impl<A: Clone> SnapshotStore<A> for InMemorySnapshotStore<A> {
    type Error = StorageError;

    fn save_snapshot(&mut self, id: StreamId, pos: EventPos, state: A) -> Result<(), Self::Error> {
        self.snapshots.insert(id, (state, pos));
        Ok(())
    }

    fn load_latest_snapshot(&self, id: StreamId) -> Result<Option<(A, EventPos)>, Self::Error> {
        Ok(self.snapshots.get(&id).cloned())
    }
}
