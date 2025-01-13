use crate::StaticFileServer;

#[tokio::main]
async fn main() -> Result<()> {
	tracing_subscriber::fmt::init();

	let server = StaticFileServer::new(PathBuf::from("dist"), 8000);

	server.serve().await?;
	Ok(())
}
