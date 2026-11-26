//! A conversation's previews, as home serves them (HOST16-S, paulgsc/server#1243;
//! paulgsc/some-ui#3558, C1582).
//!
//! After the green light the agent proposes the video's signature by the
//! rules the owner approved, and where a rule is new or in doubt it offers a
//! preview: two or three muted, looping variants, each with what is in it
//! called out, one recommended. The owner watches them on the phone and
//! picks one, which sets a rule, or says what else they want (the owner's
//! guard on paulgsc/some-ui#3521: the owner never sets a value).
//!
//! **The record is the one source.** The offer is the agent's `offer` event
//! and the pick the owner's `approve` event, both in the conversation's
//! record (`conversation_repo`). Only the clips are files, since generated
//! assets live only on home and are served by URL (C1029):
//!
//! ```text
//! renders/conversation/<subject>/<conversation>/<preview>/<variant>.mp4
//! ```
//!
//! They are the subject's, as the record is: a conversation's id is unique
//! only within its subject, so every read takes the subject and finds the
//! folder under it (`subject-<hex>`, an id no person chose), and another
//! subject's clips are a `404`. Home keeps an offer only once every one of
//! its clips is there ([`unrendered`]), so the renderer writes the clips and
//! the agent offers after. Every id is a `showId`
//! (`conversation_repo::step::is_id`), so no name reaches a path but as one
//! plain segment.
//!
//! Nothing prunes a conversation's clips: the record names each one an
//! approval was given on, and the owner may watch it again.

use conversation_repo::step::{is_id, Approval, Offer, State};
use serde::Serialize;
use std::path::PathBuf;

/// Where the renderer keeps each conversation's clips:
/// `${STORAGE}/renders/conversation`.
pub const RENDERS_FOLDER: &str = "/app/dbs/renders/conversation";

/// The folder a subject's conversations' clips are in: [`Folder::home`] on
/// home, a temporary folder in tests.
#[derive(Debug, Clone)]
pub struct Folder(pub PathBuf);

impl Folder {
	/// Home's.
	#[must_use]
	pub fn home() -> Self {
		Self(PathBuf::from(RENDERS_FOLDER))
	}

	/// The clip of `variant` of `preview` of `subject`'s `conversation`, when
	/// every name is an id.
	#[must_use]
	pub fn video(&self, subject: &str, conversation: &str, preview: &str, variant: &str) -> Option<PathBuf> {
		[subject, conversation, preview, variant]
			.iter()
			.all(|name| is_id(name))
			.then(|| self.0.join(subject).join(conversation).join(preview).join(String::from(variant) + ".mp4"))
	}
}

/// Why home refuses an offer the record's step would take: a clip it names
/// is not on home.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "why", rename_all = "kebab-case")]
pub enum Unrendered {
	OfferUnrendered { offer: String, variant: String },
}

/// The first variant of `offer` whose clip home does not hold for
/// `subject`'s `conversation`, if any.
///
/// # Errors
/// The drive's error, but a clip that is not there.
pub async fn unrendered(folder: &Folder, subject: &str, conversation: &str, offer: &Offer) -> std::io::Result<Option<Unrendered>> {
	for variant in &offer.variants {
		let missing = Unrendered::OfferUnrendered {
			offer: offer.preview.clone(),
			variant: variant.variant.clone(),
		};
		let Some(clip) = folder.video(subject, conversation, &offer.preview, &variant.variant) else {
			return Ok(Some(missing));
		};
		if !tokio::fs::try_exists(&clip).await? {
			return Ok(Some(missing));
		}
	}
	Ok(None)
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
	/// Its clip's path on home, under the API.
	pub video: String,
}

/// The owner's standing pick on a preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Approved {
	pub chose: conversation_repo::step::Choice,
}

impl From<&Approval> for Approved {
	fn from(approval: &Approval) -> Self {
		Self { chose: approval.chose.clone() }
	}
}

/// One preview a conversation offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Preview {
	pub preview: String,
	/// The card's heading: what the preview bundles.
	pub about: String,
	pub recommended: String,
	/// Two or three, in the offer's order.
	pub variants: Vec<Variant>,
	/// The owner's standing pick, once the record holds one.
	pub approved: Option<Approved>,
}

/// `GET /conversations/:id/previews`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Previews {
	/// In the order offered.
	pub previews: Vec<Preview>,
	/// Each offer whose clips home no longer holds, by id: offered, but not
	/// shown, since its videos would not play.
	pub unreadable: Vec<String>,
}

/// The previews `subject`'s `conversation` offers, from its replayed record
/// (`state`): each offer with its clips' URLs and its standing approval.
///
/// # Errors
/// The drive's error reading a clip's folder.
pub async fn previews(folder: &Folder, subject: &str, conversation: &str, state: &State) -> std::io::Result<Previews> {
	let mut out = Previews {
		previews: Vec::new(),
		unreadable: Vec::new(),
	};
	for offer in state.offers() {
		if unrendered(folder, subject, conversation, offer).await?.is_some() {
			out.unreadable.push(offer.preview.clone());
			continue;
		}
		let variants = offer
			.variants
			.iter()
			.map(|v| Variant {
				video: String::from(crate::API_V1_BASE_PATH) + "/conversations/" + conversation + "/previews/" + &offer.preview + "/" + &v.variant + "/video",
				variant: v.variant.clone(),
				title: v.title.clone(),
				rule: v.rule.clone(),
				callouts: v.callouts.clone(),
			})
			.collect();
		out.previews.push(Preview {
			preview: offer.preview.clone(),
			about: offer.about.clone(),
			recommended: offer.recommended.clone(),
			variants,
			approved: state.standing(&offer.preview).map(Approved::from),
		});
	}
	Ok(out)
}

#[cfg(test)]
pub(crate) mod tests;
