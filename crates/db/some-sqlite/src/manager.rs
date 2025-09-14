use crate::client::SqliteRepository;
use crate::traits::*;
use serde::de::Error as SerdeDeError;
use sqlx::{sqlite::SqlitePool, Row, Transaction};
use std::pin::Pin;
use std::str::FromStr;

/// Main database manager that orchestrates everything
pub struct SqliteDatabaseManager {
	pool: SqlitePool,
}

impl SqliteDatabaseManager {
	/// Create a new database manager with the given configuration
	pub async fn new(config: DatabaseConfig) -> Result<Self> {
		let mut options = sqlx::sqlite::SqliteConnectOptions::from_str(&config.database_url).map_err(|e| SqliteTemplateError::Database(sqlx::Error::Configuration(e.into())))?;

		options = options.create_if_missing(true);

		let pool = sqlx::sqlite::SqlitePoolOptions::new()
			.max_connections(config.max_connections.unwrap_or(10))
			.min_connections(config.min_connections.unwrap_or(1))
			.acquire_timeout(config.acquire_timeout.unwrap_or(std::time::Duration::from_secs(30)))
			.idle_timeout(config.idle_timeout)
			.connect_with(options)
			.await?;

		Ok(Self { pool })
	}

	/// Create a new database manager from an existing pool
	pub fn from_pool(pool: SqlitePool) -> Self {
		Self { pool }
	}

	/// Get a repository for a specific entity type
	pub fn repository<E>(&self) -> SqliteRepository<E>
	where
		E: Entity + for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin,
	{
		SqliteRepository::new(self.pool.clone())
	}

	/// Initialize the database schema for multiple entities
	pub async fn initialize_schemas<S>(&self, schemas: Vec<S>) -> Result<()>
	where
		S: Schema,
	{
		let mut tx = self.pool.begin().await?;

		for _schema in schemas {
			// Create the main table
			sqlx::query(&S::create_table_sql()).execute(&mut *tx).await?;

			// Create indexes
			for index_sql in S::indexes() {
				sqlx::query(index_sql).execute(&mut *tx).await?;
			}

			// Run setup SQL (triggers, etc.)
			for setup_sql in S::setup_sql() {
				sqlx::query(setup_sql).execute(&mut *tx).await?;
			}
		}

		tx.commit().await?;
		Ok(())
	}

	/// Run database migrations
	pub async fn migrate(&self, migrations: Vec<&str>) -> Result<()> {
		let mut tx = self.pool.begin().await?;

		for migration in migrations {
			sqlx::query(migration).execute(&mut *tx).await?;
		}

		tx.commit().await?;
		Ok(())
	}

	/// Get the underlying connection pool
	pub fn pool(&self) -> &SqlitePool {
		&self.pool
	}

	/// Close the database connection pool
	pub async fn close(&self) {
		self.pool.close().await;
	}

	/// Execute operations within a transaction
	pub async fn with_transaction<F, R>(&self, f: F) -> Result<R>
	where
		F: for<'t> FnOnce(Transaction<'t, sqlx::Sqlite>) -> Pin<Box<dyn std::future::Future<Output = Result<(R, Transaction<'t, sqlx::Sqlite>)>> + Send + 't>>,
		R: Send,
	{
		let tx = self.pool.begin().await?;

		let fut = f(tx);
		let (result, tx) = fut.await?;

		tx.commit().await.map_err(|e| SqliteTemplateError::Database(e.into()))?;

		Ok(result)
	}

	/// Begin a transaction and return it for manual management
	pub async fn begin(&self) -> Result<Transaction<'_, sqlx::Sqlite>> {
		Ok(self.pool.begin().await?)
	}
}

// Remove the problematic TransactionManager implementation since
// SQLite transactions can't be Send across await points safely
// Instead, we'll use the closure-based approach above

/// Repository implementation that works within a transaction
/// Note: This is not Send/Sync because SQLite transactions are not thread-safe
pub struct SqliteTransactionRepository<'tx, E> {
	tx: &'tx mut Transaction<'tx, sqlx::Sqlite>,
	_phantom: std::marker::PhantomData<E>,
}

impl<'tx, E> SqliteTransactionRepository<'tx, E> {
	pub fn new(tx: &'tx mut Transaction<'tx, sqlx::Sqlite>) -> Self {
		Self {
			tx,
			_phantom: std::marker::PhantomData,
		}
	}

	/// Create a new entity within the transaction
	pub async fn create<N>(&mut self, entity: N) -> Result<E>
	where
		N: NewEntity<Entity = E> + serde::Serialize + Send,
		E: Entity + for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin + serde::Serialize,
	{
		let json_value = serde_json::to_value(&entity)?;
		let object = json_value
			.as_object()
			.ok_or_else(|| SqliteTemplateError::Serialization(SerdeDeError::custom("Entity must serialize to JSON object")))?;

		let columns: Vec<String> = object.keys().cloned().collect();
		let placeholders = columns.iter().map(|_| "?").collect::<Vec<_>>().join(", ");

		let sql = format!("INSERT INTO {} ({}) VALUES ({}) RETURNING *", N::table_name(), columns.join(", "), placeholders);

		let mut query = sqlx::query_as::<_, E>(&sql);
		for column in &columns {
			if let Some(value) = object.get(column) {
				let query_value = json_value_to_query_value(value);
				query = bind_value_tx(query, query_value);
			}
		}

		let result = query.fetch_one(&mut **self.tx).await?;
		Ok(result)
	}

	/// Find entity by ID within the transaction
	pub async fn find_by_id(&mut self, id: &E::Id) -> Result<Option<E>>
	where
		E: Entity + for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin,
	{
		let sql = format!("SELECT * FROM {} WHERE {} = ?", E::table_name(), E::pk_column());

		let result = sqlx::query_as::<_, E>(&sql).bind(id.to_string()).fetch_optional(&mut **self.tx).await?;

		Ok(result)
	}

	/// Update entity within the transaction
	pub async fn update(&mut self, entity: &E) -> Result<E>
	where
		E: Entity + for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin + serde::Serialize,
	{
		let json_value = serde_json::to_value(entity)?;
		let object = json_value
			.as_object()
			.ok_or_else(|| SqliteTemplateError::Serialization(SerdeDeError::custom("Entity must serialize to JSON object")))?;

		let mut set_clauses = Vec::new();
		let mut bindings = Vec::new();

		for (key, value) in object {
			if key != E::pk_column() {
				set_clauses.push(format!("{} = ?", key));
				bindings.push(json_value_to_query_value(value));
			}
		}

		if set_clauses.is_empty() {
			return Err(SqliteTemplateError::Validation("No fields to update".to_string()));
		}

		let sql = format!("UPDATE {} SET {} WHERE {} = ? RETURNING *", E::table_name(), set_clauses.join(", "), E::pk_column());

		let mut query = sqlx::query_as::<_, E>(&sql);
		for binding in bindings {
			query = bind_value_tx(query, binding);
		}
		query = query.bind(entity.id().to_string());

		let result = query.fetch_one(&mut **self.tx).await?;
		Ok(result)
	}

	/// Delete entity by ID within the transaction
	pub async fn delete_by_id(&mut self, id: &E::Id) -> Result<u64>
	where
		E: Entity,
	{
		let sql = format!("DELETE FROM {} WHERE {} = ?", E::table_name(), E::pk_column());

		let result = sqlx::query(&sql).bind(id.to_string()).execute(&mut **self.tx).await?;

		Ok(result.rows_affected())
	}

	/// Find all entities within the transaction
	pub async fn find_all(&mut self) -> Result<Vec<E>>
	where
		E: Entity + for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin,
	{
		let sql = format!("SELECT * FROM {}", E::table_name());
		let results = sqlx::query_as::<_, E>(&sql).fetch_all(&mut **self.tx).await?;
		Ok(results)
	}

	/// Count entities with conditions within the transaction
	pub async fn count(&mut self, params: QueryParams) -> Result<i64>
	where
		E: Entity,
	{
		let query_builder = QueryBuilder::new(E::table_name());
		let (sql, bindings) = query_builder.build_count(params)?;

		let mut query = sqlx::query(&sql);
		for binding in bindings {
			query = bind_query_value(query, binding);
		}

		let row = query.fetch_one(&mut **self.tx).await?;
		let count: i64 = row.try_get(0)?;
		Ok(count)
	}

	/// Check if entity exists within the transaction
	pub async fn exists(&mut self, id: &E::Id) -> Result<bool>
	where
		E: Entity,
	{
		let sql = format!("SELECT 1 FROM {} WHERE {} = ? LIMIT 1", E::table_name(), E::pk_column());

		let result = sqlx::query(&sql).bind(id.to_string()).fetch_optional(&mut **self.tx).await?;

		Ok(result.is_some())
	}

	/// Execute a custom query within the transaction
	pub async fn query_as<T>(&mut self, sql: &str) -> Result<Vec<T>>
	where
		T: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin,
	{
		let results = sqlx::query_as::<_, T>(sql).fetch_all(&mut **self.tx).await?;
		Ok(results)
	}

	/// Execute a custom query with parameters within the transaction
	pub async fn query_as_with_params<T>(&mut self, sql: &str, params: Vec<QueryValue>) -> Result<Vec<T>>
	where
		T: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin,
	{
		let mut query = sqlx::query_as::<_, T>(sql);
		for param in params {
			query = bind_value_tx(query, param);
		}
		let results = query.fetch_all(&mut **self.tx).await?;
		Ok(results)
	}
}

// Helper functions and query builder
pub struct QueryBuilder {
	table_name: String,
}

impl QueryBuilder {
	pub fn new(table_name: &str) -> Self {
		Self {
			table_name: table_name.to_string(),
		}
	}

	pub fn build_select(&self, params: QueryParams) -> Result<(String, Vec<QueryValue>)> {
		let mut sql = format!("SELECT * FROM {}", self.table_name);
		let mut bindings = Vec::new();

		if !params.conditions.is_empty() {
			let (where_clause, where_bindings) = self.build_where_clause(&params.conditions)?;
			sql.push_str(&format!(" WHERE {}", where_clause));
			bindings.extend(where_bindings);
		}

		if !params.order_by.is_empty() {
			let order_clause = params
				.order_by
				.iter()
				.map(|o| format!("{} {}", o.column, if o.ascending { "ASC" } else { "DESC" }))
				.collect::<Vec<_>>()
				.join(", ");
			sql.push_str(&format!(" ORDER BY {}", order_clause));
		}

		if let Some(limit) = params.limit {
			sql.push_str(&format!(" LIMIT {}", limit));
		}

		if let Some(offset) = params.offset {
			sql.push_str(&format!(" OFFSET {}", offset));
		}

		Ok((sql, bindings))
	}

	pub fn build_count(&self, params: QueryParams) -> Result<(String, Vec<QueryValue>)> {
		let mut sql = format!("SELECT COUNT(*) FROM {}", self.table_name);
		let mut bindings = Vec::new();

		if !params.conditions.is_empty() {
			let (where_clause, where_bindings) = self.build_where_clause(&params.conditions)?;
			sql.push_str(&format!(" WHERE {}", where_clause));
			bindings.extend(where_bindings);
		}

		Ok((sql, bindings))
	}

	pub fn build_delete(&self, params: QueryParams) -> Result<(String, Vec<QueryValue>)> {
		let mut sql = format!("DELETE FROM {}", self.table_name);
		let mut bindings = Vec::new();

		if !params.conditions.is_empty() {
			let (where_clause, where_bindings) = self.build_where_clause(&params.conditions)?;
			sql.push_str(&format!(" WHERE {}", where_clause));
			bindings.extend(where_bindings);
		}

		Ok((sql, bindings))
	}

	fn build_where_clause(&self, conditions: &[QueryCondition]) -> Result<(String, Vec<QueryValue>)> {
		let mut clauses = Vec::new();
		let mut bindings = Vec::new();

		for condition in conditions {
			let (clause, mut condition_bindings) = self.build_condition_clause(condition)?;
			clauses.push(clause);
			bindings.append(&mut condition_bindings);
		}

		Ok((clauses.join(" AND "), bindings))
	}

	fn build_condition_clause(&self, condition: &QueryCondition) -> Result<(String, Vec<QueryValue>)> {
		match condition {
			QueryCondition::Eq(column, value) => Ok((format!("{} = ?", column), vec![value.clone()])),
			QueryCondition::Ne(column, value) => Ok((format!("{} != ?", column), vec![value.clone()])),
			QueryCondition::Gt(column, value) => Ok((format!("{} > ?", column), vec![value.clone()])),
			QueryCondition::Gte(column, value) => Ok((format!("{} >= ?", column), vec![value.clone()])),
			QueryCondition::Lt(column, value) => Ok((format!("{} < ?", column), vec![value.clone()])),
			QueryCondition::Lte(column, value) => Ok((format!("{} <= ?", column), vec![value.clone()])),
			QueryCondition::In(column, values) => {
				let placeholders = values.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
				Ok((format!("{} IN ({})", column, placeholders), values.clone()))
			}
			QueryCondition::Like(column, pattern) => Ok((format!("{} LIKE ?", column), vec![QueryValue::String(pattern.clone())])),
			QueryCondition::IsNull(column) => Ok((format!("{} IS NULL", column), vec![])),
			QueryCondition::IsNotNull(column) => Ok((format!("{} IS NOT NULL", column), vec![])),
			QueryCondition::And(conditions) => {
				let mut clauses = Vec::new();
				let mut bindings = Vec::new();
				for condition in conditions {
					let (clause, mut condition_bindings) = self.build_condition_clause(condition)?;
					clauses.push(clause);
					bindings.append(&mut condition_bindings);
				}
				Ok((format!("({})", clauses.join(" AND ")), bindings))
			}
			QueryCondition::Or(conditions) => {
				let mut clauses = Vec::new();
				let mut bindings = Vec::new();
				for condition in conditions {
					let (clause, mut condition_bindings) = self.build_condition_clause(condition)?;
					clauses.push(clause);
					bindings.append(&mut condition_bindings);
				}
				Ok((format!("({})", clauses.join(" OR ")), bindings))
			}
		}
	}
}

pub fn json_value_to_query_value(value: &serde_json::Value) -> QueryValue {
	match value {
		serde_json::Value::String(s) => QueryValue::String(s.clone()),
		serde_json::Value::Number(n) => {
			if let Some(i) = n.as_i64() {
				QueryValue::Integer(i)
			} else if let Some(f) = n.as_f64() {
				QueryValue::Float(f)
			} else {
				QueryValue::String(n.to_string())
			}
		}
		serde_json::Value::Bool(b) => QueryValue::Boolean(*b),
		serde_json::Value::Null => QueryValue::Null,
		_ => QueryValue::String(value.to_string()),
	}
}

fn bind_value_tx<'q, E>(
	query: sqlx::query::QueryAs<'q, sqlx::Sqlite, E, sqlx::sqlite::SqliteArguments<'q>>,
	value: QueryValue,
) -> sqlx::query::QueryAs<'q, sqlx::Sqlite, E, sqlx::sqlite::SqliteArguments<'q>>
where
	E: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow>,
{
	match value {
		QueryValue::String(s) => query.bind(s),
		QueryValue::Integer(i) => query.bind(i),
		QueryValue::Float(f) => query.bind(f),
		QueryValue::Boolean(b) => query.bind(b),
		QueryValue::Null => query.bind(None::<String>),
	}
}

fn bind_query_value<'q>(
	query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
	value: QueryValue,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
	match value {
		QueryValue::String(s) => query.bind(s),
		QueryValue::Integer(i) => query.bind(i),
		QueryValue::Float(f) => query.bind(f),
		QueryValue::Boolean(b) => query.bind(b),
		QueryValue::Null => query.bind(None::<String>),
	}
}
