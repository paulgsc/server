use axum::extract::Path;
use nest::http::Error;

pub async fn get(Path(id): Path<i64>) -> Result<(), Error> {
	println!("this is the path id: {id}");
	Ok(())
}
