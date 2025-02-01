use crate::FileHostError;
use axum::extract::Path;
use axum::Json;

pub async fn get(Path(id): Path<i64>) -> Result<Json<&'static str>, FileHostError> {
	println!("this is the path id: {id}");
	Ok(Json("hello world"))
}
