use anyhow::Result;
use axum::Router;
use std::path::PathBuf;
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::services::ServeDir;
pub mod handlers;
pub mod routes;

pub struct StaticFileServer {
	dist_path: PathBuf,
	port: u16,
}

impl StaticFileServer {
	pub fn new(dist_path: PathBuf, port: u16) -> Self {
		Self { dist_path, port }
	}

	pub async fn serve(self) -> Result<()> {
		let serve_dir = ServeDir::new(&self.dist_path);

		let app = Router::new()
			.nest_service("/", serve_dir)
			.layer(ServiceBuilder::new().layer(tower_http::trace::TraceLayer::new_for_http()));

		let addr = format!("127.0.0.1:{}", self.port);
		let listener = TcpListener::bind(&addr).await?;
		tracing::debug!("serving static files from {:?} on {}", self.dist_path, listener.local_addr()?);

		axum::serve(listener, app).await?;
		Ok(())
	}
}
