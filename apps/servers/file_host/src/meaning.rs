//! The take library's meaning index: the model home embeds a take's words
//! with (paulgsc/some-ui#2590, ST32; docs/take-library.md, "Where the
//! meaning index lives").
//!
//! A static embedding model, `Model2Vec`'s `potion-multilingual-128M`: a
//! tokenizer (bge-m3's `SentencePiece` unigram with its precompiled
//! normalizer, read by the `tokenizers` crate) and one row of 256 numbers
//! per token (a `safetensors` matrix). A text's vector is the mean of its
//! known tokens' rows, made unit length as `Model2Vec` makes it
//! (`config.json`'s `normalize`), so making one is a lookup per token: it
//! holds no thread and needs no worker of its own.
//!
//! - **Where it is used.** The transcriber (`transcribe::pass`) embeds a
//!   take's words in the transaction that keeps its hearing, and catches up
//!   on the takes heard before the index existed; `GET /voice-notes/takes?q=`
//!   embeds the asked words on the request's path (`handlers::voice`).
//!   `voice_repo::meaning` keeps and reads the vectors.
//! - **Nothing it makes reaches a log line.** Neither the words, the
//!   asked words nor a vector: [`StaticModel::embed`] logs nothing, and
//!   what fails at start names a file, never a text.
//! - **Mounted, never downloaded.** The model's three files sit under
//!   [`DIR`], mounted from `/models` as whisper's model is
//!   (`infra/compose/file_host.yml`, `nix/whisper-manager`'s `whisper
//!   fetch`). Off unless `MEANING_ENABLED`; on, the server refuses to start
//!   without the model, and without the transcriber, whose pass writes the
//!   index.
//!
//! **What it costs.** The matrix is about 512 MB, held for as long as the
//! server runs (500,353 tokens × 256 × 4 bytes); the tokenizer about
//! 20 MB. Loading reads both once, at start.

use crate::Config;
use anyhow::Context as _;
use safetensors::{tensor::Dtype, SafeTensors};
use std::path::Path;
use std::sync::Arc;
use tokenizers::Tokenizer;
use tracing::info;
use voice_repo::meaning::{Embed, DIMENSIONS};

/// Where compose mounts the model (`infra/compose/file_host.yml`) and
/// `nix/whisper-manager` puts it.
pub const DIR: &str = "/models/potion-multilingual-128M";

/// The model's name, stored beside each vector it makes
/// (`voice_note_meaning.model`).
pub const MODEL: &str = "potion-multilingual-128M";

/// The tensor the matrix is stored as in `model.safetensors`.
const EMBEDDINGS: &str = "embeddings";

/// A `Model2Vec` static embedding model, loaded.
pub struct StaticModel {
	name: String,
	tokenizer: Tokenizer,
	/// The matrix's bytes: one row of [`DIMENSIONS`] little-endian `f32`s per
	/// token id, in order.
	rows: Vec<u8>,
	/// The unknown token, which carries no meaning and is left out of the
	/// mean, as `Model2Vec` leaves it out.
	unknown: Option<u32>,
	/// Whether a vector is made unit length (`config.json`'s `normalize`).
	normalize: bool,
}

impl std::fmt::Debug for StaticModel {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("StaticModel").field("name", &self.name).finish_non_exhaustive()
	}
}

impl StaticModel {
	/// The model in `dir`: `tokenizer.json`, `model.safetensors` and
	/// `config.json`, as `Model2Vec` saves one.
	///
	/// # Errors
	/// A file that is not there or does not read as its part of a `Model2Vec`
	/// model of [`DIMENSIONS`] dimensions.
	pub fn load(dir: &Path, name: &str) -> anyhow::Result<Self> {
		let read =
			|file: &str| std::fs::read(dir.join(file)).with_context(|| String::from("meaning::DIR has no readable ") + file + ": run `nix develop .#whisper -c whisper fetch`");
		Self::from_parts(name, &read("tokenizer.json")?, read("model.safetensors")?, &read("config.json")?)
	}

	/// The model from its three files' bytes.
	///
	/// # Errors
	/// As [`Self::load`].
	pub fn from_parts(name: &str, tokenizer: &[u8], matrix: Vec<u8>, config: &[u8]) -> anyhow::Result<Self> {
		let config: serde_json::Value = serde_json::from_slice(config).context("config.json is not JSON")?;
		anyhow::ensure!(
			config.get("hidden_dim").and_then(serde_json::Value::as_u64) == u64::try_from(DIMENSIONS).ok(),
			"config.json's hidden_dim is not 256"
		);
		let normalize = config.get("normalize").and_then(serde_json::Value::as_bool).unwrap_or(false);

		let spec: serde_json::Value = serde_json::from_slice(tokenizer).context("tokenizer.json is not JSON")?;
		let tokenizer = Tokenizer::from_bytes(tokenizer).map_err(|_| anyhow::anyhow!("tokenizer.json does not read as a tokenizer"))?;
		let unknown = unknown(&spec, &tokenizer);

		let (header, rows, start) = {
			let (header, metadata) = SafeTensors::read_metadata(&matrix).map_err(|_| anyhow::anyhow!("model.safetensors does not read as safetensors"))?;
			let info = metadata.info(EMBEDDINGS).context("model.safetensors holds no `embeddings` tensor")?;
			anyhow::ensure!(info.dtype == Dtype::F32, "model.safetensors' embeddings are not f32");
			let [rows, dimensions] = info.shape[..] else {
				anyhow::bail!("model.safetensors' embeddings are not a matrix");
			};
			anyhow::ensure!(dimensions == DIMENSIONS, "model.safetensors' embeddings are not 256 wide");
			(header, rows, info.data_offsets.0)
		};
		anyhow::ensure!(tokenizer.get_vocab_size(true) <= rows, "model.safetensors has fewer rows than the tokenizer has tokens");
		// Keep only the matrix's own bytes: the eight-byte length and the
		// header go, and nothing is copied twice.
		let mut rows_bytes = matrix;
		rows_bytes.drain(..8 + header + start);
		rows_bytes.truncate(rows * DIMENSIONS * 4);
		Ok(Self {
			name: name.to_owned(),
			tokenizer,
			rows: rows_bytes,
			unknown,
			normalize,
		})
	}

	/// Token `id`'s row, or `None` past the matrix.
	fn row(&self, id: u32) -> Option<&[u8]> {
		let width = DIMENSIONS * 4;
		let start = usize::try_from(id).ok()?.checked_mul(width)?;
		self.rows.get(start..start.checked_add(width)?)
	}
}

impl Embed for StaticModel {
	fn model(&self) -> &str {
		&self.name
	}

	fn embed(&self, text: &str) -> Vec<f32> {
		let mut sum = vec![0.0_f32; DIMENSIONS];
		let mut count = 0.0_f32;
		// A text the tokenizer cannot read is one the model knows nothing of.
		let ids = self.tokenizer.encode_fast(text, false).map(|encoding| encoding.get_ids().to_vec()).unwrap_or_default();
		for row in ids.into_iter().filter(|id| Some(*id) != self.unknown).filter_map(|id| self.row(id)) {
			for (total, bytes) in sum.iter_mut().zip(row.chunks_exact(4)) {
				*total += f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
			}
			count += 1.0;
		}
		if count == 0.0 {
			return sum;
		}
		// `Model2Vec`'s `encode`: the mean, then divided by its length plus
		// 1e-32, so a mean of nothing stays nothing.
		let length = if self.normalize { voice_repo::meaning::norm(&sum) / count + 1e-32 } else { 1.0 };
		for x in &mut sum {
			*x = *x / count / length;
		}
		sum
	}
}

/// The tokenizer's unknown token: a unigram model's `unk_id`, or a word
/// model's `unk_token`.
fn unknown(spec: &serde_json::Value, tokenizer: &Tokenizer) -> Option<u32> {
	let model = spec.get("model")?;
	if let Some(id) = model.get("unk_id").and_then(serde_json::Value::as_u64) {
		return u32::try_from(id).ok();
	}
	model.get("unk_token").and_then(serde_json::Value::as_str).and_then(|token| tokenizer.token_to_id(token))
}

/// The model, or `None` when the deployment has not turned the meaning
/// index on.
#[derive(Clone, Debug)]
pub struct MeaningContext {
	pub model: Arc<StaticModel>,
}

impl MeaningContext {
	/// Validate the meaning index's configuration and load its model, once,
	/// at startup.
	///
	/// Off is fine and logged: the library searches words. On without the
	/// transcriber, or without a model that loads, is a hard error, as
	/// `TRANSCRIBE_ENABLED` without its model is: the only symptom of an
	/// index that is never written is a search that quietly says `words`.
	///
	/// # Errors
	/// `MEANING_ENABLED` without `TRANSCRIBE_ENABLED`, or with a model that
	/// does not load ([`StaticModel::load`]).
	pub fn from_config(config: &Config) -> anyhow::Result<Option<Self>> {
		if !config.meaning_enabled {
			info!("the take library's meaning index is not enabled; a search is by words");
			return Ok(None);
		}
		anyhow::ensure!(
			config.transcribe_enabled,
			"MEANING_ENABLED is set but TRANSCRIBE_ENABLED is not: the transcriber's pass writes the meaning index"
		);
		let model = StaticModel::load(Path::new(DIR), MODEL)?;
		info!(model = MODEL, "the take library's meaning index is enabled");
		Ok(Some(Self { model: Arc::new(model) }))
	}
}

#[cfg(test)]
pub(crate) mod tests {
	use super::StaticModel;
	use voice_repo::meaning::{cosine, norm, Embed, DIMENSIONS};

	/// A unigram tokenizer of seven pieces, shaped as bge-m3's is (a
	/// `Metaspace` pre-tokenizer, an `unk_id`), and nothing else.
	const TOKENIZER: &str = r#"{
		"version": "1.0", "truncation": null, "padding": null,
		"added_tokens": [
			{"id": 0, "content": "[PAD]", "single_word": false, "lstrip": false, "rstrip": false, "normalized": false, "special": true},
			{"id": 1, "content": "[UNK]", "single_word": false, "lstrip": false, "rstrip": false, "normalized": false, "special": true}
		],
		"normalizer": {"type": "Lowercase"},
		"pre_tokenizer": {"type": "Metaspace", "replacement": "▁", "prepend_scheme": "always", "split": true},
		"post_processor": null, "decoder": null,
		"model": {"type": "Unigram", "unk_id": 1, "byte_fallback": false, "vocab": [
			["[PAD]", 0.0], ["[UNK]", 0.0], ["▁brand", -1.0], ["▁logo", -1.0],
			["▁한국어", -1.0], ["▁the", -1.0], ["s", -2.0]
		]}
	}"#;

	const CONFIG: &str = r#"{"model_type": "model2vec", "hidden_dim": 256, "normalize": true}"#;

	/// Each token's row: `brand` and `logo` point along the first axis (one
	/// twice as long as the other), the Korean word along the second, `the`
	/// a little along both, `s` along the third, and the unknown token along
	/// the fourth, which no vector may ever show.
	fn rows() -> Vec<[f32; 4]> {
		vec![
			[0.0, 0.0, 0.0, 0.0],
			[0.0, 0.0, 0.0, 9.0],
			[2.0, 0.0, 0.0, 0.0],
			[1.0, 0.0, 0.0, 0.0],
			[0.0, 1.0, 0.0, 0.0],
			[0.1, 0.1, 0.0, 0.0],
			[0.0, 0.0, 1.0, 0.0],
		]
	}

	/// The fixture's matrix, as a `.safetensors` file: an eight-byte header
	/// length, the JSON header, then the rows, each `DIMENSIONS` wide.
	fn matrix() -> Vec<u8> {
		let rows = rows();
		let header =
			String::from(r#"{"embeddings":{"dtype":"F32","shape":["#) + &rows.len().to_string() + ",256],\"data_offsets\":[0," + &(rows.len() * DIMENSIONS * 4).to_string() + "]}}";
		let mut bytes = u64::try_from(header.len()).unwrap().to_le_bytes().to_vec();
		bytes.extend_from_slice(header.as_bytes());
		for row in rows {
			let mut wide = [0.0_f32; DIMENSIONS];
			wide[..4].copy_from_slice(&row);
			bytes.extend(wide.iter().flat_map(|x| x.to_le_bytes()));
		}
		bytes
	}

	/// The fixture model, as the transcriber's and the library's tests use it.
	pub(crate) fn fixture() -> StaticModel {
		StaticModel::from_parts("fixture", TOKENIZER.as_bytes(), matrix(), CONFIG.as_bytes()).unwrap()
	}

	fn first(v: &[f32]) -> [f32; 4] {
		[v[0], v[1], v[2], v[3]]
	}

	/// Tokenized, each known token's row looked up, their mean, unit length.
	#[test]
	fn a_vector_is_the_unit_mean_of_its_known_tokens_rows() {
		let model = fixture();
		let v = model.embed("Brand LOGO");
		assert_eq!(v.len(), DIMENSIONS);
		assert!((norm(&v) - 1.0).abs() < 1e-6, "unit length");
		assert!((v[0] - 1.0).abs() < 1e-6 && v[1..].iter().all(|x| x.abs() < f32::EPSILON), "the mean of (2, 0) and (1, 0)");

		let v = model.embed("brand 한국어");
		let half = std::f32::consts::FRAC_1_SQRT_2;
		assert!((v[0] - 2.0 / 5.0_f32.sqrt()).abs() < 1e-6 && (v[1] - 1.0 / 5.0_f32.sqrt()).abs() < 1e-6, "{:?}", first(&v));
		assert!(cosine(&v, &model.embed("logo 한국어")) < 1.0 && cosine(&v, &model.embed("logo 한국어")) > half);
	}

	/// An unknown piece carries no meaning, and words the model knows none
	/// of make no vector at all.
	#[test]
	fn unknown_tokens_are_left_out_and_nothing_known_is_nothing() {
		let model = fixture();
		let (with, without) = (model.embed("brand zzz"), model.embed("brand"));
		assert!(with.iter().zip(&without).all(|(a, b)| (a - b).abs() < f32::EPSILON), "the unknown row never shows");
		assert!(model.embed("zzz qqq").iter().all(|x| x.abs() < f32::EPSILON));
		assert!(model.embed("").iter().all(|x| x.abs() < f32::EPSILON));
		assert_eq!(model.model(), "fixture");
	}

	/// The unigram splits a word it has no piece for into pieces it has.
	#[test]
	fn a_word_is_split_into_the_pieces_the_model_knows() {
		let model = fixture();
		let brands = model.embed("brands");
		assert!(brands[0] > 0.0 && brands[2] > 0.0, "brand + s: {:?}", first(&brands));
	}

	#[test]
	fn a_model_that_does_not_fit_is_refused_by_what_is_wrong() {
		let refused = |tokenizer: &str, matrix: Vec<u8>, config: &str| StaticModel::from_parts("m", tokenizer.as_bytes(), matrix, config.as_bytes()).unwrap_err().to_string();
		assert_eq!(refused(TOKENIZER, matrix(), r#"{"hidden_dim": 512}"#), "config.json's hidden_dim is not 256");
		assert_eq!(refused("{}", matrix(), CONFIG), "tokenizer.json does not read as a tokenizer");
		assert_eq!(refused(TOKENIZER, vec![1, 2, 3], CONFIG), "model.safetensors does not read as safetensors");
		let mut short = matrix();
		short.truncate(short.len() - DIMENSIONS * 4);
		assert_eq!(refused(TOKENIZER, short, CONFIG), "model.safetensors does not read as safetensors");
		let missing = std::env::temp_dir().join("no-such-meaning-model");
		let err = StaticModel::load(&missing, "m").unwrap_err().to_string();
		assert!(err.starts_with("meaning::DIR has no readable tokenizer.json"), "{err}");
	}
}
