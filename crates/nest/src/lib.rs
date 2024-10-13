pub mod config;

pub mod http;
// lib.rs or http/mod.rs
use axum::Router;
use sqlx::SqlitePool;
use std::sync::Arc;
use std::collections::HashMap;
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::add_extension::AddExtensionLayer;
use tower_http::trace::TraceLayer;
use anyhow::Result;
use crate::config::Config;

pub trait MultiDbHandler: Send + Sync + 'static {
    fn create_routes(&self, db_name: &str, pool: SqlitePool) -> Router;
}

#[derive(Clone)]
pub struct ApiContext {
    config: Arc<Config>,
    dbs: HashMap<String, SqlitePool>,
}

pub struct ApiBuilder {
    config: Config,
    dbs: HashMap<String, SqlitePool>,
    handlers: Vec<Box<dyn MultiDbHandler>>,
}

impl ApiBuilder {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            dbs: HashMap::new(),
            handlers: Vec::new(),
        }
    }

    pub fn add_db(&mut self, name: String, pool: SqlitePool) -> &mut Self {
        self.dbs.insert(name, pool);
        self
    }

    pub fn add_handler<H: MultiDbHandler + 'static>(&mut self, handler: H) -> &mut Self {
        self.handlers.push(Box::new(handler));
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

        let app = app.layer(
            ServiceBuilder::new()
                .layer(AddExtensionLayer::new(context))
                .layer(TraceLayer::new_for_http()),
        );

        let listener = TcpListener::bind("127.0.0.1:8000").await?;
        tracing::debug!("listening on {}", listener.local_addr()?);
        axum::serve(listener, app).await?;

        Ok(())
    }
}
