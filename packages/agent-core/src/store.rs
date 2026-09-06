use crate::{AgentEvent, AgentSession, SessionId};
use rusqlite::{params, Connection};
use std::path::Path;

#[derive(Debug)]
pub enum StoreError {
    Database(rusqlite::Error),
    Serialization(serde_json::Error),
    DuplicateSequence {
        session_id: SessionId,
        sequence: u64,
    },
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "agent store database error: {error}"),
            Self::Serialization(error) => {
                write!(formatter, "agent store serialization error: {error}")
            }
            Self::DuplicateSequence {
                session_id,
                sequence,
            } => write!(
                formatter,
                "session {session_id} already contains event sequence {sequence}"
            ),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

pub trait AgentEventStore {
    fn create_session(&mut self, session: &AgentSession) -> Result<(), StoreError>;
    fn ensure_session(&mut self, session: &AgentSession) -> Result<(), StoreError>;
    fn append(&mut self, event: &AgentEvent) -> Result<(), StoreError>;
    fn events(&self, session_id: &SessionId) -> Result<Vec<AgentEvent>, StoreError>;
    fn sessions(&self) -> Result<Vec<AgentSession>, StoreError>;
}

pub struct SqliteEventStore {
    connection: Connection,
}

impl SqliteEventStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let connection = Connection::open(path)?;
        let mut store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    pub fn in_memory() -> Result<Self, StoreError> {
        let connection = Connection::open_in_memory()?;
        let mut store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<(), StoreError> {
        self.connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS agent_sessions (
                 id TEXT PRIMARY KEY,
                 created_at_ms INTEGER NOT NULL,
                 payload TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS agent_events (
                 id TEXT PRIMARY KEY,
                 session_id TEXT NOT NULL REFERENCES agent_sessions(id) ON DELETE CASCADE,
                 sequence INTEGER NOT NULL,
                 recorded_at_ms INTEGER NOT NULL,
                 payload TEXT NOT NULL,
                 UNIQUE(session_id, sequence)
             );
             CREATE INDEX IF NOT EXISTS agent_events_session_sequence
                 ON agent_events(session_id, sequence);",
        )?;
        Ok(())
    }
}

impl AgentEventStore for SqliteEventStore {
    fn create_session(&mut self, session: &AgentSession) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT INTO agent_sessions (id, created_at_ms, payload) VALUES (?1, ?2, ?3)",
            params![
                session.id.to_string(),
                session.created_at_ms as i64,
                serde_json::to_string(session)?
            ],
        )?;
        Ok(())
    }

    fn ensure_session(&mut self, session: &AgentSession) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT OR IGNORE INTO agent_sessions (id, created_at_ms, payload) VALUES (?1, ?2, ?3)",
            params![
                session.id.to_string(),
                session.created_at_ms as i64,
                serde_json::to_string(session)?
            ],
        )?;
        Ok(())
    }

    fn append(&mut self, event: &AgentEvent) -> Result<(), StoreError> {
        let result = self.connection.execute(
            "INSERT INTO agent_events (id, session_id, sequence, recorded_at_ms, payload)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                event.id.to_string(),
                event.session_id.to_string(),
                event.sequence as i64,
                event.recorded_at_ms as i64,
                serde_json::to_string(event)?
            ],
        );
        match result {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(error, _)) if error.extended_code == 2067 => {
                Err(StoreError::DuplicateSequence {
                    session_id: event.session_id.clone(),
                    sequence: event.sequence,
                })
            }
            Err(error) => Err(StoreError::Database(error)),
        }
    }

    fn events(&self, session_id: &SessionId) -> Result<Vec<AgentEvent>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT payload FROM agent_events WHERE session_id = ?1 ORDER BY sequence ASC",
        )?;
        let payloads = statement
            .query_map([session_id.to_string()], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        payloads
            .into_iter()
            .map(|payload| serde_json::from_str(&payload).map_err(StoreError::from))
            .collect()
    }

    fn sessions(&self) -> Result<Vec<AgentSession>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM agent_sessions ORDER BY created_at_ms DESC, id DESC")?;
        let payloads = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        payloads
            .into_iter()
            .map(|payload| serde_json::from_str(&payload).map_err(StoreError::from))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentEventKind;

    #[test]
    fn stores_and_replays_session_events_in_sequence() {
        let mut store = SqliteEventStore::in_memory().expect("open store");
        let session = AgentSession::new("/workspace", Some("Inspect repository".to_string()));
        store.create_session(&session).expect("create session");
        store
            .append(&AgentEvent::new(
                session.id.clone(),
                1,
                AgentEventKind::UserMessageReceived {
                    content: "inspect this repository".to_string(),
                },
            ))
            .expect("append user message");
        store
            .append(&AgentEvent::new(
                session.id.clone(),
                2,
                AgentEventKind::FinalResponseProduced {
                    content: "done".to_string(),
                },
            ))
            .expect("append response");

        let events = store.events(&session.id).expect("replay events");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, 1);
        assert_eq!(events[1].sequence, 2);
        assert_eq!(store.sessions().expect("list sessions"), vec![session]);
    }

    #[test]
    fn rejects_duplicate_sequence_numbers() {
        let mut store = SqliteEventStore::in_memory().expect("open store");
        let session = AgentSession::new("/workspace", None);
        store.create_session(&session).expect("create session");
        let event = AgentEvent::new(
            session.id.clone(),
            1,
            AgentEventKind::CheckpointSaved {
                summary: "checkpoint".to_string(),
            },
        );
        store.append(&event).expect("append event");

        assert!(matches!(
            store.append(&AgentEvent::new(
                session.id.clone(),
                1,
                AgentEventKind::TaskCompleted {
                    task_id: "task-1".to_string(),
                },
            )),
            Err(StoreError::DuplicateSequence { sequence: 1, .. })
        ));
    }
}
