use crate::traits::*;
use async_trait::async_trait;
use serde::Serialize;
use sqlx::{sqlite::SqlitePool, Row, Sqlite};
use std::marker::PhantomData;

/// SQLite-specific repository implementation
pub struct SqliteRepository<E> {
	pool: SqlitePool,
	_phantom: PhantomData<E>,
}

impl<E> SqliteRepository<E> {
	pub fn new(pool: SqlitePool) -> Self {
		Self { pool, _phantom: PhantomData }
	}
}

#[async_trait]
impl<E> Repository<E> for SqliteRepository<E>
where
	E: Entity + for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin + Serialize + Sync,
{
	async fn create<N>(&self, entity: N) -> Result<E>
	where
		N: NewEntity<Entity = E> + Serialize + Send,
	{
		let mut conn = self.pool.acquire().await?;
		self.create_with_executor(&mut *conn, entity).await
	}

	async fn create_batch<N>(&self, entities: Vec<N>) -> Result<Vec<E>>
	where
		N: NewEntity<Entity = E> + Serialize + Send,
	{
		if entities.is_empty() {
			return Ok(vec![]);
		}

		let mut tx = self.pool.begin().await?;
		let mut results = Vec::with_capacity(entities.len());
		for entity in entities {
			let row = self.create_with_executor(&mut *tx, entity).await?;
			results.push(row);
		}
		tx.commit().await?;
		Ok(results)
	}

	async fn find_by_id(&self, id: &E::Id) -> Result<Option<E>> {
		let sql = format!("SELECT * FROM {} WHERE {} = ?", E::table_name(), E::pk_column());
		let row = sqlx::query_as::<_, E>(&sql).bind(id.to_string()).fetch_optional(&self.pool).await?;
		Ok(row)
	}

	async fn find_by(&self, params: QueryParams) -> Result<Vec<E>> {
		let builder = QueryBuilder::new(E::table_name());
		let (sql, bindings) = builder.build_select(params)?;
		let mut query = sqlx::query_as::<_, E>(&sql);
		for b in bindings {
			query = bind_query_value_as(query, b);
		}
		Ok(query.fetch_all(&self.pool).await?)
	}

	async fn find_all(&self) -> Result<Vec<E>> {
		let sql = format!("SELECT * FROM {}", E::table_name());
		let rows = sqlx::query_as::<_, E>(&sql).fetch_all(&self.pool).await?;
		Ok(rows)
	}

	async fn update(&self, entity: &E) -> Result<E>
	where
		E: Serialize,
	{
		let mut conn = self.pool.acquire().await?;
		self.update_with_executor(&mut *conn, entity).await
	}

	async fn update_batch(&self, entities: Vec<&E>) -> Result<Vec<E>>
	where
		E: Serialize,
	{
		if entities.is_empty() {
			return Ok(vec![]);
		}

		let mut tx = self.pool.begin().await?;
		let mut results = Vec::with_capacity(entities.len());
		for entity in entities {
			let row = self.update_with_executor(&mut *tx, entity).await?;
			results.push(row);
		}
		tx.commit().await?;
		Ok(results)
	}

	async fn delete_by_id(&self, id: &E::Id) -> Result<u64> {
		let sql = format!("DELETE FROM {} WHERE {} = ?", E::table_name(), E::pk_column());
		let result = sqlx::query(&sql).bind(id.to_string()).execute(&self.pool).await?;
		Ok(result.rows_affected())
	}

	async fn delete_by(&self, params: QueryParams) -> Result<u64> {
		let builder = QueryBuilder::new(E::table_name());
		let (sql, bindings) = builder.build_delete(params)?;
		let mut query = sqlx::query(&sql);
		for b in bindings {
			query = bind_query_value(query, b);
		}
		let result = query.execute(&self.pool).await?;
		Ok(result.rows_affected())
	}

	async fn delete_batch(&self, ids: Vec<&E::Id>) -> Result<u64> {
		if ids.is_empty() {
			return Ok(0);
		}
		let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
		let sql = format!("DELETE FROM {} WHERE {} IN ({})", E::table_name(), E::pk_column(), placeholders);
		let mut query = sqlx::query(&sql);
		for id in ids {
			query = query.bind(id.to_string());
		}
		let result = query.execute(&self.pool).await?;
		Ok(result.rows_affected())
	}

	async fn count(&self, params: QueryParams) -> Result<i64> {
		let builder = QueryBuilder::new(E::table_name());
		let (sql, bindings) = builder.build_count(params)?;
		let mut query = sqlx::query(&sql);
		for b in bindings {
			query = bind_query_value(query, b);
		}
		let row = query.fetch_one(&self.pool).await?;
		let count: i64 = row.try_get(0)?;
		Ok(count)
	}

	async fn exists(&self, id: &E::Id) -> Result<bool> {
		let sql = format!("SELECT 1 FROM {} WHERE {} = ? LIMIT 1", E::table_name(), E::pk_column());
		let row = sqlx::query(&sql).bind(id.to_string()).fetch_optional(&self.pool).await?;
		Ok(row.is_some())
	}
}

// --- Internal helpers ---

impl<E> SqliteRepository<E>
where
	E: Entity + for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin + Serialize + Sync,
{
	async fn create_with_executor<N>(&self, exec: &mut sqlx::SqliteConnection, entity: N) -> Result<E>
	where
		N: NewEntity<Entity = E> + Serialize + Send,
	{
		let (columns, values) = entity.columns_and_values();
		let placeholders = columns.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
		let sql = format!("INSERT INTO {} ({}) VALUES ({}) RETURNING *", N::table_name(), columns.join(", "), placeholders);

		let mut query = sqlx::query_as::<_, E>(&sql);
		for value in values {
			query = bind_query_value_as(query, value);
		}

		let row = query.fetch_one(exec).await?;
		Ok(row)
	}

	async fn update_with_executor(&self, exec: &mut sqlx::SqliteConnection, entity: &E) -> Result<E> {
		let (columns, values) = entity.columns_and_values();

		let mut set_clauses = Vec::new();
		let mut bindings = Vec::new();

		for (col, val) in columns.iter().zip(values.iter()) {
			if col != &E::pk_column() {
				set_clauses.push(format!("{} = ?", col));
				bindings.push(val.clone());
			}
		}

		if set_clauses.is_empty() {
			return Err(SqliteTemplateError::Validation("No fields to update".to_string()));
		}

		let sql = format!("UPDATE {} SET {} WHERE {} = ? RETURNING *", E::table_name(), set_clauses.join(", "), E::pk_column());

		let mut query = sqlx::query_as::<_, E>(&sql);
		for b in bindings {
			query = bind_query_value_as(query, b);
		}
		query = query.bind(entity.id().to_string());

		let row = query.fetch_one(exec).await?;
		Ok(row)
	}
}

// --- Bind QueryValue helper ---
fn bind_query_value<'q>(
	query: sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
	value: QueryValue,
) -> sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
	match value {
		QueryValue::String(s) => query.bind(s),
		QueryValue::Integer(i) => query.bind(i),
		QueryValue::Float(f) => query.bind(f),
		QueryValue::Boolean(b) => query.bind(b),
		QueryValue::Null => query.bind(None::<String>),
	}
}

// --- Overloaded bind function for QueryAs ---
fn bind_query_value_as<'q, O>(
	query: sqlx::query::QueryAs<'q, Sqlite, O, sqlx::sqlite::SqliteArguments<'q>>,
	value: QueryValue,
) -> sqlx::query::QueryAs<'q, Sqlite, O, sqlx::sqlite::SqliteArguments<'q>>
where
	O: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin,
{
	match value {
		QueryValue::String(s) => query.bind(s),
		QueryValue::Integer(i) => query.bind(i),
		QueryValue::Float(f) => query.bind(f),
		QueryValue::Boolean(b) => query.bind(b),
		QueryValue::Null => query.bind(None::<String>),
	}
}

/// Dynamic SQL builder for QueryParams
struct QueryBuilder {
	table_name: String,
}

impl QueryBuilder {
	fn new(table_name: &str) -> Self {
		Self {
			table_name: table_name.to_string(),
		}
	}

	fn build_select(&self, params: QueryParams) -> Result<(String, Vec<QueryValue>)> {
		let mut sql = format!("SELECT * FROM {}", self.table_name);
		let (where_clause, bindings) = self.build_conditions(&params.conditions)?;
		if !where_clause.is_empty() {
			sql.push_str(" WHERE ");
			sql.push_str(&where_clause);
		}

		if !params.order_by.is_empty() {
			let order_clause = params
				.order_by
				.iter()
				.map(|o| format!("{} {}", o.column, if o.ascending { "ASC" } else { "DESC" }))
				.collect::<Vec<_>>()
				.join(", ");
			sql.push_str(" ORDER BY ");
			sql.push_str(&order_clause);
		}

		if let Some(limit) = params.limit {
			sql.push_str(&format!(" LIMIT {}", limit));
			if let Some(offset) = params.offset {
				sql.push_str(&format!(" OFFSET {}", offset));
			}
		}

		Ok((sql, bindings))
	}

	fn build_delete(&self, params: QueryParams) -> Result<(String, Vec<QueryValue>)> {
		let mut sql = format!("DELETE FROM {}", self.table_name);
		let (where_clause, bindings) = self.build_conditions(&params.conditions)?;
		if !where_clause.is_empty() {
			sql.push_str(" WHERE ");
			sql.push_str(&where_clause);
		}
		Ok((sql, bindings))
	}

	fn build_count(&self, params: QueryParams) -> Result<(String, Vec<QueryValue>)> {
		let mut sql = format!("SELECT COUNT(*) FROM {}", self.table_name);
		let (where_clause, bindings) = self.build_conditions(&params.conditions)?;
		if !where_clause.is_empty() {
			sql.push_str(" WHERE ");
			sql.push_str(&where_clause);
		}
		Ok((sql, bindings))
	}

	fn build_conditions(&self, conditions: &[QueryCondition]) -> Result<(String, Vec<QueryValue>)> {
		if conditions.is_empty() {
			return Ok(("".into(), vec![]));
		}

		let mut clauses = Vec::new();
		let mut bindings = Vec::new();

		for cond in conditions {
			let (clause, b) = self.build_condition(cond)?;
			clauses.push(clause);
			bindings.extend(b);
		}

		Ok((clauses.join(" AND "), bindings))
	}

	fn build_condition(&self, condition: &QueryCondition) -> Result<(String, Vec<QueryValue>)> {
		match condition {
			QueryCondition::Eq(col, val) => Ok((format!("{} = ?", col), vec![val.clone()])),
			QueryCondition::Ne(col, val) => Ok((format!("{} != ?", col), vec![val.clone()])),
			QueryCondition::Gt(col, val) => Ok((format!("{} > ?", col), vec![val.clone()])),
			QueryCondition::Gte(col, val) => Ok((format!("{} >= ?", col), vec![val.clone()])),
			QueryCondition::Lt(col, val) => Ok((format!("{} < ?", col), vec![val.clone()])),
			QueryCondition::Lte(col, val) => Ok((format!("{} <= ?", col), vec![val.clone()])),
			QueryCondition::In(col, vals) => {
				if vals.is_empty() {
					return Ok(("1=0".into(), vec![])); // safe false
				}
				let placeholders = vals.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
				Ok((format!("{} IN ({})", col, placeholders), vals.clone()))
			}
			QueryCondition::Like(col, pat) => Ok((format!("{} LIKE ?", col), vec![QueryValue::String(pat.clone())])),
			QueryCondition::IsNull(col) => Ok((format!("{} IS NULL", col), vec![])),
			QueryCondition::IsNotNull(col) => Ok((format!("{} IS NOT NULL", col), vec![])),
			QueryCondition::And(conds) => {
				let mut clauses = Vec::new();
				let mut bindings = Vec::new();
				for c in conds {
					let (cl, b) = self.build_condition(c)?;
					clauses.push(format!("({})", cl));
					bindings.extend(b);
				}
				Ok((clauses.join(" AND "), bindings))
			}
			QueryCondition::Or(conds) => {
				let mut clauses = Vec::new();
				let mut bindings = Vec::new();
				for c in conds {
					let (cl, b) = self.build_condition(c)?;
					clauses.push(format!("({})", cl));
					bindings.extend(b);
				}
				Ok((clauses.join(" OR "), bindings))
			}
		}
	}
}
