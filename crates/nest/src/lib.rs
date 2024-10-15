pub mod config;

pub mod http;

use crate::http::Error;
use anyhow::{Context, Result};
use sqlx::sqlite::SqlitePoolOptions;
use tracing_subscriber::{filter::EnvFilter, fmt::format::JsonFields, util::SubscriberInitExt, Layer};

use crate::config::Config;
use axum::Router;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::add_extension::AddExtensionLayer;
use tower_http::trace::TraceLayer;

pub trait MigrationHandler: Send + Sync + 'static {
	fn run_migrations<'a>(&'a self, pool: &'a SqlitePool) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>;
}

pub trait MultiDbHandler: Send + Sync + 'static {
	fn create_routes(&self, db_name: &str, pool: SqlitePool) -> Router;
}

#[derive(Clone)]
pub struct ApiContext {
	config: Arc<Config>,
	dbs: HashMap<String, SqlitePool>,
}

pub trait Run<M: MigrationHandler> {
	type Future: Future<Output = Result<()>> + Send + 'static;

	fn run<I>(config: Config, handlers: I, migration_handler: M) -> Self::Future
	where
		I: IntoIterator<Item = Box<dyn MultiDbHandler + Send>> + Send + 'static;
}

pub struct ApiBuilder<M: MigrationHandler> {
	config: Config,
	dbs: HashMap<String, SqlitePool>,
	handlers: Vec<Box<dyn MultiDbHandler>>,
	migration_handler: M,
}

impl<M: MigrationHandler> ApiBuilder<M> {
	pub fn new(config: Config, migration_handler: M) -> Self {
		Self {
			config,
			dbs: HashMap::new(),
			handlers: Vec::new(),
			migration_handler,
		}
	}

	pub fn add_db(&mut self, name: String, pool: SqlitePool) -> &mut Self {
		self.dbs.insert(name, pool);
		self
	}

	pub fn add_handler(&mut self, handler: Box<dyn MultiDbHandler>) -> &mut Self {
		self.handlers.push(handler);
		self
	}

	pub async fn serve(self) -> Result<()> {
		let context = ApiContext {
			config: Arc::new(self.config),
			dbs: self.dbs.clone(),
		};
		let mut app = Router::new();
		for (db_name, db_pool) in &self.dbs {
			for handler in &self.handlers {
				app = app.merge(handler.create_routes(db_name, db_pool.clone()));
			}
		}

		let app = app.layer(ServiceBuilder::new().layer(AddExtensionLayer::new(context)).layer(TraceLayer::new_for_http()));
		let listener = TcpListener::bind("127.0.0.1:8000").await?;
		tracing::debug!("listening on {}", listener.local_addr()?);
		axum::serve(listener, app).await?;
		Ok(())
	}
}

impl<M: MigrationHandler> Run<M> for ApiBuilder<M> {
	type Future = Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>;

	fn run<I>(config: Config, handlers: I, migration_handler: M) -> Self::Future
	where
		I: IntoIterator<Item = Box<dyn MultiDbHandler + Send>> + Send + 'static,
	{
		Box::pin(async move {
			let mut api_builder = Self::new(config.clone(), migration_handler);

			for (i, db_url) in config.database_urls.split(',').enumerate() {
				let db_name = format!("db_{}", i + 1);
				let db_pool = SqlitePoolOptions::new()
					.max_connections(5)
					.connect(db_url)
					.await
					.context(format!("could not connect to {}", db_url))?;

				// Run migrations using the passed migration handler
				api_builder.migration_handler.run_migrations(&db_pool).await?;

				api_builder.add_db(db_name, db_pool);
			}

			for handler in handlers {
				api_builder.add_handler(handler);
			}

			api_builder.serve().await?;
			Ok(())
		})
	}
}

pub fn init_tracing(config: &Config) -> Option<()> {
	use std::str::FromStr;
	use tracing_subscriber::layer::SubscriberExt;

	let filter = EnvFilter::from_str(config.rust_log.as_deref()?).unwrap();

	tracing_subscriber::registry()
		.with(if config.log_json {
			Box::new(
				tracing_subscriber::fmt::layer()
					.fmt_fields(JsonFields::default())
					.event_format(tracing_subscriber::fmt::format().json().flatten_event(true).with_span_list(false))
					.with_filter(filter),
			) as Box<dyn Layer<_> + Send + Sync>
		} else {
			Box::new(
				tracing_subscriber::fmt::layer()
					.event_format(tracing_subscriber::fmt::format().pretty())
					.with_filter(filter),
			)
		})
		.init();
	None
}
