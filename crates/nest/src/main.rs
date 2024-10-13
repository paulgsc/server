use anyhow::Context;
use clap::Parser;

use tracing_subscriber::{
    filter::EnvFilter, fmt::format::JsonFields, util::SubscriberInitExt, Layer
};

use nest::config::Config;
use nest::ApiBuilder;
use nest::http::routes::BrowserTabsHandler;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
	let config = Config::parse();
    init_tracing(&config);

    let mut api_builder = ApiBuilder::new(config.clone());
    
    for (i, db_url) in config.database_urls.split(',').enumerate() {
        let db_name = format!("db_{}", i + 1);
        let db_pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(db_url)
            .await
            .context(format!("could not connect to {}", db_url))?;

        sqlx::migrate!().run(&db_pool).await?;
        api_builder.add_db(db_name, db_pool);
    }

    api_builder.add_handler(BrowserTabsHandler);
    api_builder.serve().await?;

	Ok(())
}

pub fn init_tracing(config: &Config) -> Option<()> {
    use std::str::FromStr;
    use tracing_subscriber::layer::SubscriberExt;

    let filter = EnvFilter::from_str(config.rust_log.as_deref()?).unwrap();

    tracing_subscriber::registry()
        .with(
            if config.log_json {
                Box::new(
                    tracing_subscriber::fmt::layer()
                    .fmt_fields(JsonFields::default())
                    .event_format(
                        tracing_subscriber::fmt::format()
                            .json()
                            .flatten_event(true)
                            .with_span_list(false),
                    )
                    .with_filter(filter),
                ) as Box<dyn Layer<_> + Send + Sync>
            } else {
                Box::new(
                    tracing_subscriber::fmt::layer()
                        .event_format(tracing_subscriber::fmt::format().pretty())
                        .with_filter(filter),
                )
            }
        )
        .init();
    None
}
