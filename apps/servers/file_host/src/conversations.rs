//! A conversation's previews, as home keeps them (HOST16-S, paulgsc/server#1243;
//! paulgsc/some-ui#3558).
//!
//! After the green light the agent proposes the video's signature by the
//! rules the owner approved, and where a rule is new or in doubt it renders a
//! preview: two or three muted, looping variants, each with what is in it
//! called out, one recommended. The owner watches them on the phone and
//! picks one, which sets a rule, or says what else they want (the owner's
//! guard on paulgsc/some-ui#3521: the owner never sets a value). The pick is
//! an `approve` event in the conversation's record (`conversation_repo`).
//!
//! Generated assets live only on home and are served by URL (C1029), so a
//! preview is files, kept no table:
//!
//! ```text
//! renders/conversation/<conversation>/<preview>/preview.json   some-ui.conversation-preview/1:
//!     { schema, preview, about, recommended, variants: [{ variant, title, rule, callouts }] }
//! renders/conversation/<conversation>/<preview>/<variant>.mp4  each variant's video
//! ```
//!
//! The renderer writes `preview.json` last, so a preview is listed once its
//! manifest is there and every variant's video beside it: before that it is
//! still being made and is not offered. A manifest that does not read is
//! named in the answer (`unreadable`), never left out unsaid. Every id is a
//! `showId` (`conversation_repo::step::is_id`), so no name from a request or
//! a manifest reaches a path but as one plain segment.
//!
//! Nothing prunes a conversation's previews: the record names each one an
//! approval was given on, and the owner may watch it again.

use conversation_repo::step::{is_id, Approval, Choice, PREVIEW_VARIANTS_MOST};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where the renderer keeps each conversation's previews:
/// `${STORAGE}/renders/conversation`.
pub const RENDERS_FOLDER: &str = "/app/dbs/renders/conversation";

/// A preview's manifest, in its folder.
pub const MANIFEST: &str = "preview.json";

/// The manifest's schema tag. Its shape is the one the phone's card renders
/// (paulgsc/some-ui `packages/video-render/src/preview-offer.ts`,
/// `PreviewOfferSchema`, #3575), named as home's files name it.
pub const MANIFEST_SCHEMA: &str = "some-ui.conversation-preview/1";

/// The largest manifest home reads.
const MANIFEST_CEILING: u64 = 64 * 1024;

/// The most callouts one variant has: "what's in it", a short list.
const CALLOUTS_MOST: usize = 12;

/// The most characters of one callout, or of a rule's name.
const CALLOUT_MOST: usize = 200;

/// The most previews one conversation lists.
const PREVIEWS_MOST: usize = 200;

/// The folder a conversation's previews are in: [`Folder::home`] on home, a
/// temporary folder in tests.
#[derive(Debug, Clone)]
pub struct Folder(pub PathBuf);

impl Folder {
	/// Home's.
	#[must_use]
	pub fn home() -> Self {
		Self(PathBuf::from(RENDERS_FOLDER))
	}

	fn of(&self, conversation: &str) -> PathBuf {
		self.0.join(conversation)
	}

	/// The video of `variant` of `preview`, when every name is an id.
	#[must_use]
	pub fn video(&self, conversation: &str, preview: &str, variant: &str) -> Option<PathBuf> {
		(is_id(conversation) && is_id(preview) && is_id(variant)).then(|| self.of(conversation).join(preview).join(String::from(variant) + ".mp4"))
	}
}

/// A manifest as the renderer writes it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
	schema: String,
	preview: String,
	/// The card's heading: what the preview bundles.
	about: String,
	recommended: String,
	variants: Vec<ManifestVariant>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestVariant {
	variant: String,
	/// The variant's name on the card.
	title: String,
	rule: String,
	callouts: Vec<String>,
}

/// One variant a preview offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Variant {
	pub variant: String,
	/// Its name on the card.
	pub title: String,
	/// The rule a pick of it sets.
	pub rule: String,
	/// What is in it, as the clip calls each out.
	pub callouts: Vec<String>,
	/// Its video's path on home, under the API.
	pub video: String,
}

/// The record's approval of a preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Approved {
	pub chose: Choice,
}

/// One preview a conversation offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Preview {
	pub preview: String,
	/// The card's heading: what the preview bundles.
	pub about: String,
	pub recommended: String,
	/// Two or three, in the manifest's order.
	pub variants: Vec<Variant>,
	/// The owner's pick, once the record holds one.
	pub approved: Option<Approved>,
}

/// `GET /conversations/:id/previews`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Previews {
	/// By id.
	pub previews: Vec<Preview>,
	/// Each preview whose manifest is there but does not read, by id.
	pub unreadable: Vec<String>,
}

fn fits(text: &str, most: usize) -> bool {
	!text.trim().is_empty() && text.trim() == text && text.chars().count() <= most
}

/// The manifest's offer, checked: its own id, two or three variants each an
/// id once, the recommended one of them, and its heading and each title,
/// rule and callout a short line.
fn offer(manifest: Manifest, preview: &str) -> Option<Offer> {
	let names: Vec<&str> = manifest.variants.iter().map(|v| v.variant.as_str()).collect();
	let good = manifest.schema == MANIFEST_SCHEMA
		&& manifest.preview == preview
		&& (2..=PREVIEW_VARIANTS_MOST).contains(&names.len())
		&& names.iter().enumerate().all(|(i, name)| is_id(name) && !names[..i].contains(name))
		&& names.contains(&manifest.recommended.as_str())
		&& fits(&manifest.about, CALLOUT_MOST)
		&& manifest
			.variants
			.iter()
			.all(|v| fits(&v.title, 64) && fits(&v.rule, 64) && (1..=CALLOUTS_MOST).contains(&v.callouts.len()) && v.callouts.iter().all(|c| fits(c, CALLOUT_MOST)));
	good.then_some(Offer {
		about: manifest.about,
		recommended: manifest.recommended,
		variants: manifest.variants,
	})
}

/// A ready preview's offer, as its manifest says it.
struct Offer {
	about: String,
	recommended: String,
	variants: Vec<ManifestVariant>,
}

/// What reading one preview's folder found.
enum Read {
	/// Still being made: no manifest yet, or a variant's video not there.
	Making,
	Unreadable,
	Ready(Offer),
}

async fn read_one(folder: &Path, preview: &str) -> std::io::Result<Read> {
	let manifest = folder.join(MANIFEST);
	let size = match tokio::fs::metadata(&manifest).await {
		Ok(meta) => meta.len(),
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Read::Making),
		Err(err) => return Err(err),
	};
	if size > MANIFEST_CEILING {
		return Ok(Read::Unreadable);
	}
	let bytes = tokio::fs::read(&manifest).await?;
	let Ok(parsed) = serde_json::from_slice::<Manifest>(&bytes) else {
		return Ok(Read::Unreadable);
	};
	let Some(offer) = offer(parsed, preview) else {
		return Ok(Read::Unreadable);
	};
	for variant in &offer.variants {
		if !tokio::fs::try_exists(folder.join(String::from(&variant.variant) + ".mp4")).await? {
			return Ok(Read::Making);
		}
	}
	Ok(Read::Ready(offer))
}

/// The previews `conversation` offers, each with the record's approval of it
/// (`approvals`, from the replayed log).
///
/// # Errors
/// The drive's error reading the folder or a manifest. A conversation home
/// holds no folder for offers none.
pub async fn previews(folder: &Folder, conversation: &str, approvals: &[Approval]) -> std::io::Result<Previews> {
	let mut out = Previews {
		previews: Vec::new(),
		unreadable: Vec::new(),
	};
	if !is_id(conversation) {
		return Ok(out);
	}
	let root = folder.of(conversation);
	let mut entries = match tokio::fs::read_dir(&root).await {
		Ok(entries) => entries,
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(out),
		Err(err) => return Err(err),
	};
	let mut ids = Vec::new();
	while let Some(entry) = entries.next_entry().await? {
		let name = entry.file_name();
		if let Some(name) = name.to_str().filter(|name| is_id(name)) {
			if entry.file_type().await?.is_dir() {
				ids.push(name.to_owned());
			}
		}
	}
	ids.sort_unstable();
	ids.truncate(PREVIEWS_MOST);
	for id in ids {
		match read_one(&root.join(&id), &id).await? {
			Read::Making => {}
			Read::Unreadable => out.unreadable.push(id),
			Read::Ready(Offer { about, recommended, variants }) => {
				let approved = approvals.iter().find(|a| a.preview == id).map(|a| Approved { chose: a.chose.clone() });
				let variants = variants
					.into_iter()
					.map(|v| Variant {
						video: String::from(crate::API_V1_BASE_PATH) + "/conversations/" + conversation + "/previews/" + &id + "/" + &v.variant + "/video",
						variant: v.variant,
						title: v.title,
						rule: v.rule,
						callouts: v.callouts,
					})
					.collect();
				out.previews.push(Preview {
					preview: id,
					about,
					recommended,
					variants,
					approved,
				});
			}
		}
	}
	Ok(out)
}

/// Why home refuses an approval the record's step would take: the preview
/// it names is not one home offers as it says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "why", rename_all = "kebab-case")]
pub enum Unoffered {
	/// Home offers no such preview of this conversation.
	PreviewUnknown { preview: String },
	/// Its `offered`, `recommended` or a variant's rule is not what home's
	/// preview offers.
	PreviewOffer { preview: String },
}

/// Whether `approval` names a preview of `conversation` home offers, exactly
/// as it offers it.
///
/// # Errors
/// The drive's error reading the preview.
pub async fn offered(folder: &Folder, conversation: &str, approval: &Approval) -> std::io::Result<Result<(), Unoffered>> {
	let preview = &approval.preview;
	let unknown = Unoffered::PreviewUnknown { preview: preview.clone() };
	if !is_id(conversation) || !is_id(preview) {
		return Ok(Err(unknown));
	}
	let Offer { recommended, variants, .. } = match read_one(&folder.of(conversation).join(preview), preview).await {
		Ok(Read::Ready(offer)) => offer,
		Ok(Read::Making | Read::Unreadable) => return Ok(Err(unknown)),
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Err(unknown)),
		Err(err) => return Err(err),
	};
	let names: Vec<&str> = variants.iter().map(|v| v.variant.as_str()).collect();
	let rule_holds = match &approval.chose {
		Choice::Variant { variant, rule } => variants.iter().any(|v| &v.variant == variant && &v.rule == rule),
		Choice::Else { .. } => true,
	};
	let same = approval.offered.iter().map(String::as_str).eq(names.iter().copied()) && approval.recommended == recommended && rule_holds;
	Ok(if same { Ok(()) } else { Err(Unoffered::PreviewOffer { preview: preview.clone() }) })
}

#[cfg(test)]
pub(crate) mod tests;
