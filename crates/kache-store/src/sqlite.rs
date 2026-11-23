use rusqlite::{Params, Row, ToSql};
use std::cell::Cell;
use std::ops::Deref;
use std::path::Path;
use std::time::Duration;

pub struct Connection {
    inner: rusqlite::Connection,
    failed: Cell<bool>,
    monitored: Cell<bool>,
}

#[derive(Clone, Copy)]
pub struct ConnectionRef<'a> {
    inner: &'a rusqlite::Connection,
    failed: Option<&'a Cell<bool>>,
}

fn record<T>(failed: Option<&Cell<bool>>, result: rusqlite::Result<T>) -> rusqlite::Result<T> {
    if result
        .as_ref()
        .is_err_and(|error| !matches!(error, rusqlite::Error::QueryReturnedNoRows))
        && let Some(failed) = failed
    {
        failed.set(true);
    }
    result
}

impl From<rusqlite::Connection> for Connection {
    fn from(inner: rusqlite::Connection) -> Self {
        Self {
            inner,
            failed: Cell::new(false),
            monitored: Cell::new(false),
        }
    }
}

impl<'a> From<&'a Connection> for ConnectionRef<'a> {
    fn from(db: &'a Connection) -> Self {
        Self {
            inner: &db.inner,
            failed: db.monitored.get().then_some(&db.failed),
        }
    }
}

impl<'a> From<&'a rusqlite::Connection> for ConnectionRef<'a> {
    fn from(inner: &'a rusqlite::Connection) -> Self {
        Self {
            inner,
            failed: None,
        }
    }
}

impl Deref for Connection {
    type Target = rusqlite::Connection;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl Deref for ConnectionRef<'_> {
    type Target = rusqlite::Connection;
    fn deref(&self) -> &Self::Target {
        self.inner
    }
}

impl Connection {
    pub(crate) fn monitor(&self) {
        self.monitored.set(true);
    }
    pub(crate) fn reusable(&self) -> bool {
        !self.failed.get() && self.inner.is_autocommit()
    }
    pub fn open(path: impl AsRef<Path>) -> rusqlite::Result<Self> {
        rusqlite::Connection::open(path).map(Self::from)
    }
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        rusqlite::Connection::open_in_memory().map(Self::from)
    }
    pub fn open_with_flags(
        path: impl AsRef<Path>,
        flags: rusqlite::OpenFlags,
    ) -> rusqlite::Result<Self> {
        rusqlite::Connection::open_with_flags(path, flags).map(Self::from)
    }
    pub fn execute<P: Params>(&self, sql: &str, params: P) -> rusqlite::Result<usize> {
        ConnectionRef::from(self).execute(sql, params)
    }
    pub fn execute_batch(&self, sql: &str) -> rusqlite::Result<()> {
        ConnectionRef::from(self).execute_batch(sql)
    }
    pub fn query_row<T, P: Params, F: FnOnce(&Row<'_>) -> rusqlite::Result<T>>(
        &self,
        sql: &str,
        params: P,
        f: F,
    ) -> rusqlite::Result<T> {
        ConnectionRef::from(self).query_row(sql, params, f)
    }
    pub fn prepare(&self, sql: &str) -> rusqlite::Result<Statement<'_>> {
        ConnectionRef::from(self).prepare(sql)
    }
    pub fn prepare_cached(&self, sql: &str) -> rusqlite::Result<Statement<'_>> {
        ConnectionRef::from(self).prepare_cached(sql)
    }
    pub fn busy_timeout(&self, timeout: Duration) -> rusqlite::Result<()> {
        ConnectionRef::from(self).busy_timeout(timeout)
    }
    pub fn busy_handler(&mut self, callback: Option<fn(i32) -> bool>) -> rusqlite::Result<()> {
        let failed = self.monitored.get().then_some(&self.failed);
        record(failed, self.inner.busy_handler(callback))
    }
    pub fn pragma_update<V: ToSql>(
        &self,
        schema: Option<&str>,
        pragma: &str,
        value: V,
    ) -> rusqlite::Result<()> {
        ConnectionRef::from(self).pragma_update(schema, pragma, value)
    }
    pub fn unchecked_transaction(&self) -> rusqlite::Result<Transaction<'_>> {
        ConnectionRef::from(self).observe();
        let inner = record(
            ConnectionRef::from(self).failed,
            self.inner.unchecked_transaction(),
        )?;
        Ok(Transaction {
            inner: Some(inner),
            parent: self,
        })
    }
    pub fn transaction_with_behavior(
        &self,
        behavior: rusqlite::TransactionBehavior,
    ) -> rusqlite::Result<Transaction<'_>> {
        let inner = ConnectionRef::from(self)
            .run(|| rusqlite::Transaction::new_unchecked(&self.inner, behavior))?;
        Ok(Transaction {
            inner: Some(inner),
            parent: self,
        })
    }
}

impl<'a> ConnectionRef<'a> {
    fn observe(self) {
        if let Some(failed) = self.failed {
            // The borrowed connection cannot be used concurrently.
            let code = unsafe { rusqlite::ffi::sqlite3_errcode(self.inner.handle()) };
            if !matches!(code, 0 | 100 | 101) {
                failed.set(true);
            }
        }
    }
    fn run<T>(self, f: impl FnOnce() -> rusqlite::Result<T>) -> rusqlite::Result<T> {
        self.observe();
        record(self.failed, f())
    }
    pub fn execute<P: Params>(self, sql: &str, params: P) -> rusqlite::Result<usize> {
        self.run(|| self.inner.execute(sql, params))
    }
    pub fn execute_batch(self, sql: &str) -> rusqlite::Result<()> {
        self.run(|| self.inner.execute_batch(sql))
    }
    pub fn query_row<T, P: Params, F: FnOnce(&Row<'_>) -> rusqlite::Result<T>>(
        self,
        sql: &str,
        params: P,
        f: F,
    ) -> rusqlite::Result<T> {
        if self.failed.is_some() {
            return self.prepare_cached(sql)?.query_row(params, f);
        }
        self.run(|| self.inner.query_row(sql, params, f))
    }
    pub fn prepare(self, sql: &str) -> rusqlite::Result<Statement<'a>> {
        Ok(Statement {
            inner: StatementKind::Direct(self.run(|| self.inner.prepare(sql))?),
            failed: self.failed,
        })
    }
    pub fn prepare_cached(self, sql: &str) -> rusqlite::Result<Statement<'a>> {
        Ok(Statement {
            inner: StatementKind::Cached(self.run(|| self.inner.prepare_cached(sql))?),
            failed: self.failed,
        })
    }
    pub fn transaction_with_behavior(
        self,
        behavior: rusqlite::TransactionBehavior,
    ) -> rusqlite::Result<RefTransaction<'a>> {
        let inner = self.run(|| rusqlite::Transaction::new_unchecked(self.inner, behavior))?;
        Ok(RefTransaction {
            inner: Some(inner),
            parent: self,
        })
    }
    pub fn busy_timeout(self, timeout: Duration) -> rusqlite::Result<()> {
        self.run(|| self.inner.busy_timeout(timeout))
    }
    pub fn pragma_update<V: ToSql>(
        self,
        schema: Option<&str>,
        pragma: &str,
        value: V,
    ) -> rusqlite::Result<()> {
        self.run(|| self.inner.pragma_update(schema, pragma, value))
    }
}

enum StatementKind<'a> {
    Direct(rusqlite::Statement<'a>),
    Cached(rusqlite::CachedStatement<'a>),
}

pub struct Statement<'a> {
    inner: StatementKind<'a>,
    failed: Option<&'a Cell<bool>>,
}

impl<'a> Statement<'a> {
    fn raw(&mut self) -> &mut rusqlite::Statement<'a> {
        match &mut self.inner {
            StatementKind::Direct(s) => s,
            StatementKind::Cached(s) => s,
        }
    }
    pub fn query_row<T, P: Params, F: FnOnce(&Row<'_>) -> rusqlite::Result<T>>(
        &mut self,
        params: P,
        f: F,
    ) -> rusqlite::Result<T> {
        let failed = self.failed;
        record(failed, self.raw().query_row(params, f))
    }
    pub fn execute<P: Params>(&mut self, params: P) -> rusqlite::Result<usize> {
        let failed = self.failed;
        record(failed, self.raw().execute(params))
    }
    pub fn query<P: Params>(&mut self, params: P) -> rusqlite::Result<Rows<'_>> {
        let failed = self.failed;
        Ok(Rows {
            inner: record(failed, self.raw().query(params))?,
            failed,
        })
    }
    pub fn query_map<T, P: Params, F: FnMut(&Row<'_>) -> rusqlite::Result<T>>(
        &mut self,
        params: P,
        f: F,
    ) -> rusqlite::Result<MappedRows<'_, F>> {
        let failed = self.failed;
        Ok(MappedRows {
            inner: record(failed, self.raw().query_map(params, f))?,
            failed,
        })
    }
}

pub struct Rows<'a> {
    inner: rusqlite::Rows<'a>,
    failed: Option<&'a Cell<bool>>,
}
impl<'a> Rows<'a> {
    pub fn advance(&mut self) -> rusqlite::Result<Option<CheckedRow<'_, 'a>>> {
        Ok(
            record(self.failed, self.inner.next())?.map(|inner| CheckedRow {
                inner,
                failed: self.failed,
            }),
        )
    }
}

pub struct CheckedRow<'r, 's> {
    inner: &'r Row<'s>,
    failed: Option<&'s Cell<bool>>,
}

impl CheckedRow<'_, '_> {
    pub fn get<I: rusqlite::RowIndex, T: rusqlite::types::FromSql>(
        &self,
        index: I,
    ) -> rusqlite::Result<T> {
        record(self.failed, self.inner.get(index))
    }
}

pub struct MappedRows<'a, F> {
    inner: rusqlite::MappedRows<'a, F>,
    failed: Option<&'a Cell<bool>>,
}
impl<T, F: FnMut(&Row<'_>) -> rusqlite::Result<T>> Iterator for MappedRows<'_, F> {
    type Item = rusqlite::Result<T>;
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|result| record(self.failed, result))
    }
}

pub struct Transaction<'a> {
    inner: Option<rusqlite::Transaction<'a>>,
    parent: &'a Connection,
}
impl Deref for Transaction<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.parent
    }
}
impl Transaction<'_> {
    pub fn commit(mut self) -> rusqlite::Result<()> {
        ConnectionRef::from(self.parent)
            .run(|| self.inner.take().expect("live transaction").commit())
    }
    pub fn rollback(mut self) -> rusqlite::Result<()> {
        ConnectionRef::from(self.parent)
            .run(|| self.inner.take().expect("live transaction").rollback())
    }
}
impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        ConnectionRef::from(self.parent).observe();
    }
}

pub struct RefTransaction<'a> {
    inner: Option<rusqlite::Transaction<'a>>,
    parent: ConnectionRef<'a>,
}
impl<'a> Deref for RefTransaction<'a> {
    type Target = ConnectionRef<'a>;
    fn deref(&self) -> &Self::Target {
        &self.parent
    }
}
impl RefTransaction<'_> {
    pub fn commit(mut self) -> rusqlite::Result<()> {
        self.parent
            .run(|| self.inner.take().expect("live transaction").commit())
    }
    pub fn rollback(mut self) -> rusqlite::Result<()> {
        self.parent
            .run(|| self.inner.take().expect("live transaction").rollback())
    }
}
impl Drop for RefTransaction<'_> {
    fn drop(&mut self) {
        self.parent.observe();
    }
}

impl<'a> From<&Transaction<'a>> for ConnectionRef<'a> {
    fn from(tx: &Transaction<'a>) -> Self {
        tx.parent.into()
    }
}
impl<'a> From<&RefTransaction<'a>> for ConnectionRef<'a> {
    fn from(tx: &RefTransaction<'a>) -> Self {
        tx.parent
    }
}
impl<'a> From<&ConnectionRef<'a>> for ConnectionRef<'a> {
    fn from(db: &ConnectionRef<'a>) -> Self {
        *db
    }
}
impl<'a> From<&'a rusqlite::Transaction<'_>> for ConnectionRef<'a> {
    fn from(tx: &'a rusqlite::Transaction<'_>) -> Self {
        Self {
            inner: tx,
            failed: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::OptionalExtension;

    #[test]
    fn missing_rows_do_not_poison_a_connection() {
        let db = Connection::open_in_memory().unwrap();
        db.monitor();
        assert_eq!(
            db.query_row("SELECT 1 WHERE 0", [], |r| r.get::<_, i64>(0))
                .optional()
                .unwrap(),
            None
        );
        assert!(db.reusable());
    }

    #[test]
    fn step_errors_survive_discarded_rows_and_later_success() {
        let db = Connection::open_in_memory().unwrap();
        db.monitor();
        {
            let mut stmt = db
                .prepare_cached("SELECT abs(-9223372036854775808)")
                .unwrap();
            let values: Vec<i64> = stmt
                .query_map([], |r| r.get(0))
                .unwrap()
                .filter_map(Result::ok)
                .collect();
            assert!(values.is_empty());
        }
        assert_eq!(
            db.query_row("SELECT 1", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(!db.reusable());
    }

    #[test]
    fn failed_commit_remains_poisoned_after_rollback() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE parent(id PRIMARY KEY); CREATE TABLE child(id REFERENCES parent(id) DEFERRABLE INITIALLY DEFERRED);").unwrap();
        db.monitor();
        let tx = db.unchecked_transaction().unwrap();
        tx.execute("INSERT INTO child VALUES(1)", []).unwrap();
        assert!(tx.commit().is_err());
        assert_eq!(
            db.query_row("SELECT 1", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(!db.reusable());
    }

    #[test]
    fn borrowed_transaction_errors_are_sticky_too() {
        let db = Connection::open_in_memory().unwrap();
        db.monitor();
        let tx = ConnectionRef::from(&db)
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        assert!(tx.execute_batch("invalid syntax").is_err());
        tx.rollback().unwrap();
        db.execute_batch("SELECT 1").unwrap();
        assert!(!db.reusable());
    }

    #[test]
    fn cursor_conversion_errors_are_sticky() {
        let db = Connection::open_in_memory().unwrap();
        db.monitor();
        {
            let mut stmt = db.prepare("SELECT 'not a number'").unwrap();
            let mut rows = stmt.query([]).unwrap();
            assert!(rows.advance().unwrap().unwrap().get::<_, i64>(0).is_err());
        }
        db.execute_batch("SELECT 1").unwrap();
        assert!(!db.reusable());
    }
}
