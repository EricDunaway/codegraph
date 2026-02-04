//! Database connection management

use crate::error::DbError;
use crate::queries::QueryBuilder;
use crate::schema::SCHEMA;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

/// Database connection wrapper
pub struct DatabaseConnection {
    conn: Connection,
    queries: QueryBuilder,
}

impl DatabaseConnection {
    /// Open a database at the given path
    pub fn open(path: &Path) -> Result<Self, DbError> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;

        let conn = Connection::open_with_flags(path, flags)?;

        // Configure pragmas for performance
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "cache_size", "-64000")?; // 64MB cache
        conn.pragma_update(None, "temp_store", "MEMORY")?;

        // Apply schema
        conn.execute_batch(SCHEMA)?;

        let queries = QueryBuilder::new(&conn)?;

        Ok(Self { conn, queries })
    }

    /// Open an in-memory database (for testing)
    pub fn open_in_memory() -> Result<Self, DbError> {
        let conn = Connection::open_in_memory()?;

        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;

        let queries = QueryBuilder::new(&conn)?;

        Ok(Self { conn, queries })
    }

    /// Get a reference to the underlying connection
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Get a reference to the query builder
    pub fn queries(&self) -> &QueryBuilder {
        &self.queries
    }

    /// Begin a transaction
    pub fn transaction(&mut self) -> Result<rusqlite::Transaction<'_>, DbError> {
        Ok(self.conn.transaction()?)
    }
}
