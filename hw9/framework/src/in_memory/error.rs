use std::error::Error;
use std::fmt;

use crate::event_sourcing::event_store::{EventPos, StreamId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    /// Запрошено чтение с позиции дальше конца стрима.
    PosOutOfRange {
        stream_id: StreamId,
        pos: EventPos,
        len: usize,
    },
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PosOutOfRange {
                stream_id,
                pos,
                len,
            } => write!(
                f,
                "position {} is past the end of stream {} ({len} events)",
                pos.0, stream_id.0
            ),
        }
    }
}

impl Error for StorageError {}
