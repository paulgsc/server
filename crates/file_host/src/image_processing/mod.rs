use std::io::Cursor;
use std::sync::Arc;

use axum::{
	extract::{Path, Query, State},
	http::{HeaderMap, StatusCode},
	response::{IntoResponse, Response},
	routing::get,
	Router,
};
use bytes::Bytes;
use fast_image_resize as fir;
use image::{DynamicImage, ImageFormat, ImageOutputFormat};
use serde::Deserialize;
use thiserror::Error;
use tokio::sync::Semaphore;

const DEFAULT_QUALITY: u8 = 85;
const DEFAULT_CACHE_CONTROL: &str = "public, max-age=86400";

#[derive(Error, Debug)]
pub enum ImageError {
	#[error("Failed to load image: {0}")]
	LoadError(#[from] image::ImageError),
	#[error("Failed to resize image: {0}")]
	ResizeError(String),
	#[error("Failed to encode image: {0}")]
	EncodeError(String),
	#[error("Image processing failed")]
	ProcessingError,
	#[error("Internal server error")]
	InternalServerError,
	#[error("Bad request: {0}")]
	BadRequest(String),
	#[error("Not found")]
	NotFound,
	#[error("Semaphore acquisition failed")]
	SemaphoreError,
}

// Convert ImageError to StatusCode
impl From<ImageError> for StatusCode {
	fn from(error: ImageError) -> Self {
		match error {
			ImageError::LoadError(_) => StatusCode::BAD_REQUEST,
			ImageError::ResizeError(_) => StatusCode::BAD_REQUEST,
			ImageError::EncodeError(_) => StatusCode::INTERNAL_SERVER_ERROR,
			ImageError::ProcessingError => StatusCode::INTERNAL_SERVER_ERROR,
			ImageError::InternalServerError => StatusCode::INTERNAL_SERVER_ERROR,
			ImageError::BadRequest(_) => StatusCode::BAD_REQUEST,
			ImageError::NotFound => StatusCode::NOT_FOUND,
			ImageError::SemaphoreError => StatusCode::INTERNAL_SERVER_ERROR,
		}
	}
}

// ===============================
// Image Transformation Options
// ===============================

#[derive(Deserialize, Debug, Clone)]
pub struct ImageTransformOptions {
	pub width: Option<u32>,
	pub height: Option<u32>,
	pub quality: Option<u8>,
	pub format: Option<String>,
	pub crop: Option<bool>,
	pub crop_gravity: Option<String>,
	pub filter: Option<String>,
	pub auto_orient: Option<bool>,
	pub grayscale: Option<bool>,
	pub blur: Option<f32>,
	pub sharpen: Option<f32>,
}

impl Default for ImageTransformOptions {
	fn default() -> Self {
		Self {
			width: None,
			height: None,
			quality: Some(DEFAULT_QUALITY),
			format: None,
			crop: Some(false),
			crop_gravity: None,
			filter: None,
			auto_orient: Some(false),
			grayscale: Some(false),
			blur: None,
			sharpen: None,
		}
	}
}

pub struct ImageProcessor {
	semaphore: Arc<Semaphore>,
}

impl ImageProcessor {
	pub fn new(max_concurrent: usize) -> Self {
		Self {
			semaphore: Arc::new(Semaphore::new(max_concurrent)),
		}
	}

	pub async fn process(&self, image_data: Bytes, options: &ImageTransformOptions) -> Result<(Bytes, ImageFormat), ImageError> {
		let _permit = self.semaphore.acquire().await.map_err(|_| ImageError::SemaphoreError)?;
		let options = options.clone(); // avoid capturing in the closure
		tokio::task::spawn_blocking(move || Self::transform_image(image_data, &options))
			.await
			.map_err(|_| ImageError::ProcessingError)?
	}

	fn transform_image(image_data: Bytes, options: &ImageTransformOptions) -> Result<(Bytes, ImageFormat), ImageError> {
		let img = image::load_from_memory(&image_data)?;
		let mut transformed = img;

		if options.auto_orient.unwrap_or(false) {
			transformed = Self::auto_orient(transformed);
		}

		if options.width.is_some() || options.height.is_some() {
			transformed = Self::resize(
				transformed,
				options.width,
				options.height,
				options.crop.unwrap_or(false),
				options.crop_gravity.as_deref(),
				options.filter.as_deref(),
			)?;
		}

		if options.grayscale.unwrap_or(false) {
			transformed = DynamicImage::ImageLuma8(transformed.to_luma8());
		}

		if let Some(sigma) = options.blur {
			if sigma > 0.0 {
				transformed = transformed.blur(sigma);
			}
		}

		if let Some(sigma) = options.sharpen {
			if sigma > 0.0 {
				transformed = transformed.unsharpen(sigma, 1);
			}
		}

		let format = Self::determine_format(&image_data, &options.format);
		let buffer = Self::encode_image(transformed, format, options.quality.unwrap_or(DEFAULT_QUALITY))?;
		Ok((Bytes::from(buffer.into_inner()), format))
	}

	fn determine_format(image_data: &Bytes, format_option: &Option<String>) -> ImageFormat {
		format_option
			.as_deref()
			.map(|f| match f.to_lowercase().as_str() {
				"jpeg" | "jpg" => ImageFormat::Jpeg,
				"png" => ImageFormat::Png,
				"webp" => ImageFormat::WebP,
				"avif" => ImageFormat::Avif,
				_ => image::guess_format(image_data).unwrap_or(ImageFormat::Jpeg),
			})
			.unwrap_or_else(|| image::guess_format(image_data).unwrap_or(ImageFormat::Jpeg))
	}

	fn encode_image(image: DynamicImage, format: ImageFormat, quality: u8) -> Result<Cursor<Vec<u8>>, ImageError> {
		let mut buffer = Cursor::new(Vec::new());
		let quality = quality.min(100);

		match format {
			ImageFormat::Jpeg => {
				let output_format = ImageOutputFormat::Jpeg(quality);
				image
					.write_to(&mut buffer, output_format)
					.map_err(|e| ImageError::EncodeError(format!("Failed to encode JPEG: {}", e)))?;
			}
			ImageFormat::Png => {
				image
					.write_to(&mut buffer, ImageOutputFormat::Png)
					.map_err(|e| ImageError::EncodeError(format!("Failed to encode PNG: {}", e)))?;
			}
			ImageFormat::WebP => {
				#[cfg(feature = "webp")]
				{
					let output_format = ImageOutputFormat::WebP(quality as f32 / 100.0);
					image
						.write_to(&mut buffer, output_format)
						.map_err(|e| ImageError::EncodeError(format!("Failed to encode WebP: {}", e)))?;
				}
				#[cfg(not(feature = "webp"))]
				{
					return Err(ImageError::EncodeError("WebP encoding not supported".to_string()));
				}
			}
			ImageFormat::Avif => {
				#[cfg(feature = "avif")]
				{
					// AVIF encoding
				}
				#[cfg(not(feature = "avif"))]
				{
					return Err(ImageError::EncodeError("AVIF encoding not supported".to_string()));
				}
			}
			_ => {
				image
					.write_to(&mut buffer, ImageOutputFormat::Png)
					.map_err(|e| ImageError::EncodeError(format!("Failed to encode image: {}", e)))?;
			}
		}
		Ok(buffer)
	}

	fn auto_orient(img: DynamicImage) -> DynamicImage {
		// Simplified auto-orient
		img
	}

	fn resize(img: DynamicImage, width: Option<u32>, height: Option<u32>, crop: bool, gravity: Option<&str>, filter_type: Option<&str>) -> Result<DynamicImage, ImageError> {
		let (orig_width, orig_height) = (img.width(), img.height());
		let (target_width, target_height) = match (width, height) {
			(Some(w), Some(h)) => (w, h),
			(Some(w), None) => {
				let ratio = w as f32 / orig_width as f32;
				let h = (orig_height as f32 * ratio).round() as u32;
				(w, h)
			}
			(None, Some(h)) => {
				let ratio = h as f32 / orig_height as f32;
				let w = (orig_width as f32 * ratio).round() as u32;
				(w, h)
			}
			(None, None) => (orig_width, orig_height),
		};

		let filter = match filter_type {
			Some("nearest") => fir::ResizeAlg::Nearest,
			Some("catmullrom") => fir::ResizeAlg::CatmullRom,
			Some("gaussian") => fir::ResizeAlg::Gaussian(2.0),
			Some("triangle") => fir::ResizeAlg::Triangle,
			_ => fir::ResizeAlg::Lanczos3,
		};

		if crop && width.is_some() && height.is_some() {
			let src_ratio = orig_width as f32 / orig_height as f32;
			let target_ratio = target_width as f32 / target_height as f32;
			let (crop_width, crop_height) = if src_ratio > target_ratio {
				let new_width = (orig_height as f32 * target_ratio).round() as u32;
				(new_width, orig_height)
			} else {
				let new_height = (orig_width as f32 / target_ratio).round() as u32;
				(orig_width, new_height)
			};

			let x_offset = match gravity {
				Some("northwest") | Some("north") | Some("northeast") => 0,
				Some("southwest") | Some("south") | Some("southeast") => orig_width.saturating_sub(crop_width),
				_ => (orig_width - crop_width) / 2,
			};

			let y_offset = match gravity {
				Some("northwest") | Some("west") | Some("southwest") => 0,
				Some("northeast") | Some("east") | Some("southeast") => orig_height.saturating_sub(crop_height),
				_ => (orig_height - crop_height) / 2,
			};

			let cropped = img.crop_imm(
				x_offset.min(orig_width - 1),
				y_offset.min(orig_height - 1),
				crop_width.min(orig_width - x_offset),
				crop_height.min(orig_height - y_offset),
			);

			let src_image = fir::Image::from_dynamic_image(&cropped).map_err(|e| ImageError::ResizeError(format!("Failed to convert image: {}", e)))?;
			let mut dst_image = fir::Image::new(target_width, target_height, src_image.pixel_type());
			let mut resizer = fir::Resizer::new(filter);
			resizer
				.resize(&src_image.view(), &mut dst_image.view_mut())
				.map_err(|e| ImageError::ResizeError(format!("Failed to resize: {}", e)))?;

			DynamicImage::from_raw_pixels(dst_image.buffer().to_vec(), target_width, target_height, dst_image.pixel_type().into())
				.ok_or_else(|| ImageError::ResizeError("Failed to create final image".to_string()))
		} else {
			let src_image = fir::Image::from_dynamic_image(&img).map_err(|e| ImageError::ResizeError(format!("Failed to convert image: {}", e)))?;
			let mut dst_image = fir::Image::new(target_width, target_height, src_image.pixel_type());
			let mut resizer = fir::Resizer::new(filter);
			resizer
				.resize(&src_image.view(), &mut dst_image.view_mut())
				.map_err(|e| ImageError::ResizeError(format!("Failed to resize: {}", e)))?;
			DynamicImage::from_raw_pixels(dst_image.buffer().to_vec(), target_width, target_height, dst_image.pixel_type().into())
				.ok_or_else(|| ImageError::ResizeError("Failed to create final image".to_string()))
		}
	}
}

// ===============================
// Image Cache Trait
// ===============================
#[async_trait::async_trait]
pub trait ImageCache: Send + Sync {
	async fn get(&self, key: &str) -> Option<Bytes>;
	async fn put(&self, key: &str, data: Bytes, ttl: Option<u64>) -> Result<(), ImageError>;
}

// ===============================
// Axum Handler
// ===============================
async fn transform_image_handler(
	Path(image_id): Path<String>,
	Query(options): Query<ImageTransformOptions>,
	State(processor): State<Arc<ImageProcessor>>,
	State(cache): State<Arc<dyn ImageCache>>,
) -> Result<Response, ImageError> {
	let cache_key = format!(
		"img:{}:w{}:h{}:q{}:fmt{}:crop{}:filter{}:gray{}:blur{}:sharp{}",
		image_id,
		options.width.unwrap_or(0),
		options.height.unwrap_or(0),
		options.quality.unwrap_or(0),
		options.format.as_deref().unwrap_or("orig"),
		options.crop.unwrap_or(false),
		options.filter.as_deref().unwrap_or("default"),
		options.grayscale.unwrap_or(false),
		options.blur.unwrap_or(0.0),
		options.sharpen.unwrap_or(0.0),
	);

	if let Some(cached_data) = cache.get(&cache_key).await {
		let format = image::guess_format(&cached_data).unwrap_or(ImageFormat::Jpeg);
		return create_image_response(cached_data, format);
	}

	let original_image = fetch_from_gdrive(&image_id).await.map_err(|_| ImageError::NotFound)?;

	let (processed_image, format) = if options.width.is_none()
		&& options.height.is_none()
		&& options.quality.is_none()
		&& options.format.is_none()
		&& !options.grayscale.unwrap_or(false)
		&& options.blur.is_none()
		&& options.sharpen.is_none()
	{
		cache.put(&cache_key, original_image.clone(), None).await?;
		(original_image, image::guess_format(&original_image).unwrap_or(ImageFormat::Jpeg))
	} else {
		let result = processor.process(original_image, &options).await?;
		cache.put(&cache_key, result.0.clone(), None).await?; // Cache the processed image
		result
	};

	create_image_response(processed_image, format)
}

// ===============================
// Response Helper
// ===============================
fn create_image_response(data: Bytes, format: ImageFormat) -> Result<Response, ImageError> {
	let content_type = match format {
		ImageFormat::Jpeg => "image/jpeg",
		ImageFormat::Png => "image/png",
		ImageFormat::WebP => "image/webp",
		ImageFormat::Avif => "image/avif",
		_ => "application/octet-stream",
	};

	let mut headers = HeaderMap::new();
	headers.insert("content-type", content_type.parse().unwrap());
	headers.insert("cache-control", DEFAULT_CACHE_CONTROL.parse().unwrap());

	Ok((headers, data).into_response())
}
