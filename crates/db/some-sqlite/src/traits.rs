use async_trait::async_trait;
use serde::Serialize;
use std::fmt::Debug;
use thiserror::Error;

/// Core error type for the SQLite template
#[derive(Debug, Error)]
pub enum SqliteTemplateError {
	#[error("Database error: {0}")]
	Database(#[from] sqlx::Error),
	#[error("Serialization error: {0}")]
	Serialization(#[from] serde_json::Error),
	#[error("Validation error: {0}")]
	Validation(String),
	#[error("Not found: {entity} with {field} = {value}")]
	NotFound { entity: String, field: String, value: String },
}

pub type Result<T> = std::result::Result<T, SqliteTemplateError>;

/// Entity trait
pub trait Entity: Clone + Debug + Send + Sync + 'static {
	type Id: Clone + Debug + Send + Sync + ToString + 'static;

	fn id(&self) -> &Self::Id;
	fn table_name() -> &'static str;
	fn pk_column() -> &'static str {
		"id"
	}
	fn columns_and_values(&self) -> (Vec<&str>, Vec<QueryValue>);
}

/// NewEntity trait
pub trait NewEntity: Clone + Debug + Send + Sync + 'static {
	type Entity: Entity;

	fn table_name() -> &'static str {
		Self::Entity::table_name()
	}
	fn columns_and_values(&self) -> (Vec<&str>, Vec<QueryValue>);
}

/// Schema definition
pub trait Schema: Send + Sync + 'static {
	fn create_table_sql() -> &'static str;
	fn indexes() -> Vec<&'static str> {
		vec![]
	}
	fn setup_sql() -> Vec<&'static str> {
		vec![]
	}
}

/// Querying
#[derive(Debug, Clone)]
pub enum QueryCondition {
	Eq(String, QueryValue),
	Ne(String, QueryValue),
	Gt(String, QueryValue),
	Gte(String, QueryValue),
	Lt(String, QueryValue),
	Lte(String, QueryValue),
	In(String, Vec<QueryValue>),
	Like(String, String),
	IsNull(String),
	IsNotNull(String),
	And(Vec<QueryCondition>),
	Or(Vec<QueryCondition>),
}

#[derive(Debug, Clone)]
pub enum QueryValue {
	String(String),
	Integer(i64),
	Float(f64),
	Boolean(bool),
	Null,
}

#[derive(Debug, Clone)]
pub struct OrderBy {
	pub column: String,
	pub ascending: bool,
}

#[derive(Debug, Clone, Default)]
pub struct QueryParams {
	pub conditions: Vec<QueryCondition>,
	pub order_by: Vec<OrderBy>,
	pub limit: Option<u32>,
	pub offset: Option<u32>,
}

/// Repository trait - Removed Sync bound since SQLite transactions are not thread-safe
#[async_trait]
pub trait Repository<E>: Send
where
	E: Entity + for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin,
{
	async fn create<N>(&self, entity: N) -> Result<E>
	where
		N: NewEntity<Entity = E> + Serialize + Send;

	async fn create_batch<N>(&self, entities: Vec<N>) -> Result<Vec<E>>
	where
		N: NewEntity<Entity = E> + Serialize + Send;

	async fn find_by_id(&self, id: &E::Id) -> Result<Option<E>>;

	async fn find_by(&self, params: QueryParams) -> Result<Vec<E>>;

	async fn find_all(&self) -> Result<Vec<E>>;

	async fn update(&self, entity: &E) -> Result<E>
	where
		E: Serialize;

	async fn update_batch(&self, entities: Vec<&E>) -> Result<Vec<E>>
	where
		E: Serialize;

	async fn delete_by_id(&self, id: &E::Id) -> Result<u64>;

	async fn delete_by(&self, params: QueryParams) -> Result<u64>;

	async fn delete_batch(&self, ids: Vec<&E::Id>) -> Result<u64>;

	async fn count(&self, params: QueryParams) -> Result<i64>;

	async fn exists(&self, id: &E::Id) -> Result<bool>;
}

/// Transaction management simplified
#[async_trait]
pub trait TransactionManager: Send + Sync {
	type Tx: Send;
	async fn begin(&self) -> Result<Self::Tx>;
}

/// Database configuration
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
	pub database_url: String,
	pub max_connections: Option<u32>,
	pub min_connections: Option<u32>,
	pub acquire_timeout: Option<std::time::Duration>,
	pub idle_timeout: Option<std::time::Duration>,
}

impl Default for DatabaseConfig {
	fn default() -> Self {
		Self {
			database_url: ":memory:".to_string(),
			max_connections: Some(10),
			min_connections: Some(1),
			acquire_timeout: Some(std::time::Duration::from_secs(30)),
			idle_timeout: Some(std::time::Duration::from_secs(600)),
		}
	}
}
