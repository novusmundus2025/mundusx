use crate::StoreError;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub content: String,
    pub tags: Vec<String>,
    pub created_at_ms: u64,
}

pub struct SqliteMemoryStore {
    path: PathBuf,
}

impl SqliteMemoryStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let store = Self {
            path: path.as_ref().to_path_buf(),
        };
        store.connection()?;
        Ok(store)
    }

    fn connection(&self) -> Result<Connection, StoreError> {
        let connection = Connection::open(&self.path)?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS agent_memories (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                tags TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS agent_memories_created_at
                ON agent_memories(created_at_ms DESC);",
        )?;
        Ok(connection)
    }

    pub fn save(&self, content: &str, tags: &[String]) -> Result<MemoryRecord, StoreError> {
        let record = MemoryRecord {
            id: Uuid::new_v4().to_string(),
            content: content.to_string(),
            tags: tags.to_vec(),
            created_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis() as u64)
                .unwrap_or(0),
        };
        self.connection()?.execute(
            "INSERT INTO agent_memories (id, content, tags, created_at_ms) VALUES (?1, ?2, ?3, ?4)",
            params![
                &record.id,
                &record.content,
                serde_json::to_string(tags)?,
                record.created_at_ms as i64
            ],
        )?;
        Ok(record)
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryRecord>, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, content, tags, created_at_ms FROM agent_memories
             WHERE lower(content) LIKE lower(?1) OR lower(tags) LIKE lower(?1)
             ORDER BY created_at_ms DESC LIMIT ?2",
        )?;
        let pattern = format!("%{}%", query);
        let rows = statement.query_map(params![pattern, limit.clamp(1, 50) as i64], |row| {
            let tags: String = row.get(2)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                tags,
                row.get::<_, i64>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (id, content, tags, created_at_ms) = row?;
            Ok(MemoryRecord {
                id,
                content,
                tags: serde_json::from_str(&tags)?,
                created_at_ms: created_at_ms as u64,
            })
        })
        .collect()
    }
}
