//! What home hands its render worker beside an essay: the essay's media, one
//! family (paulgsc/some-ui#3311's follow-on, C820, C830).
//!
//! An essay names things only home holds. Each is one [`Kind`] of a closed
//! set:
//!
//! - [`Kind::Takes`]: the owner's takes, by note id ([`takes`], WALK7,
//!   paulgsc/server#1052);
//! - [`Kind::Snips`]: screens of the app from the snip library, by snip id
//!   ([`snips`], SNIPS4, paulgsc/some-ui#3114);
//! - [`Kind::ClipFrames`]: a clip's frames, by clip, side and file
//!   ([`clip_frames`], the Clips Library's `clip_frame`);
//! - [`Kind::Pages`]: public pages, by the capture run that took each and
//!   its file ([`pages`], paulgsc/some-ui#3395);
//! - [`Kind::Walks`]: our own apps in motion, by the `Snips Capture` run
//!   that walked each and its app ([`walks`], paulgsc/some-ui#3476).
//!
//! A snip, a clip frame, a page and a walk are each a cut list's `screen` picture
//! (`{"kind": "screen", "of": {"from": "snips" | "clips" | "pages" | "walks", …}}`,
//! some-ui's SNIPS4), drawn the same way; they are kinds of their own here
//! because home holds them in different places and hands them in different
//! folders.
//!
//! What a kind is, is named here once: which of an essay's parts name it
//! ([`named`]), the folder it goes in, its refusal's reason, and how home
//! reads one item of it. Every rule below is one rule for every kind:
//!
//! - **Written whole, before the essay.** A cards essay's `<key>.json` is
//!   given only once every kind it names is ready, each written to
//!   `<key>.<kind>/` ([`Kind::folder`]) through `.<key>.<kind>.partial`
//!   ([`write`]), so the worker never sees an essay without its media, nor a
//!   half-written folder.
//! - **Wait, refuse or give.** Each item is ready, not ready yet (a take not
//!   heard yet: the essay waits and is asked again next pass), or never to
//!   be given as home holds it now ([`Why`]). A refusal outranks a wait, so
//!   an essay that can never be given says so at once. The refusal said is
//!   the first kind's in [`Kind::ALL`]'s order that refuses, and within a
//!   kind a missing item outranks an unheard one: an unheard take and a
//!   missing clip frame say `take-unheard`. A refusal is said by its kind's reason ([`Refusal::reason`]),
//!   with the items by id, never what they hold.
//! - **No copy outlives its need.** A kind's folder goes once its essay's
//!   video is made, and an essay given with a take or clip frame home no
//!   longer holds is withdrawn ([`still_held`]; the caller's tidy,
//!   `cards_essays::pass::tidy`). A snip is never a reason to take one back:
//!   it is no one's record, and its copy goes with the video. Nor is a page,
//!   nor a walk.
//! - **Nothing is ever written to home's own record here.** Each kind only
//!   reads: `clip_frame` rows stay as they were kept, and the owner's takes
//!   stay where `voice_repo` keeps them.
//! - **Counts and kinds, never bytes.** Nothing here logs.
//!
//! A Rust Rounds or interview essay plays takes too, by its week's final
//! record, which has its own rules (it waits for a take not yet home, and is
//! withdrawn for one deleted): it shares [`takes::hand_off`], one take's
//! files, and writes them to its own `<week>/takes/` as before.

pub mod clip_frames;
pub mod pages;
pub mod snips;
pub mod takes;
pub mod walks;

use serde_json::Value;
use sqlx::SqliteConnection;
use std::path::{Component, Path, PathBuf};

/// One kind of an essay's media, in the order an essay is checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
	Takes,
	Snips,
	ClipFrames,
	Pages,
	Walks,
}

impl Kind {
	/// Every kind, in the order an essay is checked and a refusal is said.
	pub const ALL: [Self; 5] = [Self::Takes, Self::Snips, Self::ClipFrames, Self::Pages, Self::Walks];

	/// Its folder's suffix beside a cards essay: `<key>.<folder>/`, the
	/// recorder's `--<folder>` (`infra/render/render-calls.sh`).
	#[must_use]
	pub const fn folder(self) -> &'static str {
		match self {
			Self::Takes => "takes",
			Self::Snips => "snips",
			Self::ClipFrames => "clips",
			Self::Pages => "pages",
			Self::Walks => "walks",
		}
	}

	/// How a refusal or a wait names `item`: by its id. A page's item also
	/// carries the scene's address and moment, which only its check reads;
	/// it is named by its run and file alone. A walk's carries its steps,
	/// and is named by its run and app alone.
	#[must_use]
	pub fn id_of(self, item: &str) -> String {
		match self {
			Self::Pages => pages::id_of(item),
			Self::Walks => walks::id_of(item),
			Self::Takes | Self::Snips | Self::ClipFrames => String::from(item),
		}
	}
}

/// Why an item is never to be given as home holds it now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Why {
	/// Home does not hold it: never sent or kept, deleted, let go, or not
	/// what an id of its kind is.
	Missing,
	/// A take home couldn't make out, or heard nothing said in.
	Unheard,
	/// A snip or page whose bundle home cannot fetch: no run of its commit
	/// or no such capture run, its artifact expired, or its bundle refused
	/// (`snips`, C875); or a page its run's bundle doesn't hold as the scene
	/// names it (C1441); or a walk its run's bundle doesn't list, or whose
	/// steps it doesn't have (C1471).
	Gone,
}

/// One file of a kind's folder: its name there (a relative path of plain
/// parts) and its bytes.
pub type File = (String, Vec<u8>);

/// What home holds of one item asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Held {
	/// Its files.
	Files(Vec<File>),
	/// A snip, a page or a walk: the key of the bundle it came from (a
	/// commit, or a run), its record as that bundle has it, and its pictures
	/// (a walk's are each of its steps; a snip or page has one). Its kind's
	/// record names it ([`snips::given`], [`pages::given`], [`walks::given`]).
	Record { bundle: String, record: Value, pictures: Vec<File> },
	/// Not ready yet: asked again next pass.
	Waiting,
	/// Never to be given as home holds it now.
	Refused(Why),
}

/// Why an essay was refused: its kind, why, and the items it is for, by id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
	pub kind: Kind,
	pub why: Why,
	pub items: Vec<String>,
}

impl Refusal {
	/// Studio › Renders' `reason` (`feeds::renders`'s home refusals).
	#[must_use]
	pub const fn reason(&self) -> &'static str {
		match (self.kind, self.why) {
			// A take is never gone: that is a snip's bundle.
			(Kind::Takes, Why::Missing | Why::Gone) => "take-missing",
			(Kind::Takes, Why::Unheard) => "take-unheard",
			(Kind::Snips, Why::Gone) => "snip-gone",
			(Kind::Snips, _) => "snip-missing",
			(Kind::ClipFrames, _) => "clip-missing",
			(Kind::Pages, Why::Gone) => "page-gone",
			(Kind::Pages, _) => "page-missing",
			(Kind::Walks, Why::Gone) => "walk-gone",
			(Kind::Walks, _) => "walk-missing",
		}
	}

	/// Its words in a failed log: why, never what an item holds.
	#[must_use]
	pub const fn words(&self) -> &'static str {
		match (self.kind, self.why) {
			(Kind::Takes, Why::Missing | Why::Gone) => "a spoken card names a take home doesn't hold (never sent, deleted, or its recording let go)",
			(Kind::Takes, Why::Unheard) => "home couldn't make out a take a spoken card plays, or heard nothing said in it, and a spoken card has no silence to play in its place",
			(Kind::Snips, Why::Gone) => "a scene shows a snip whose bundle GitHub no longer has (no run of its commit, its artifact expired, or its bundle didn't check out): run Snips Capture again at that commit",
			(Kind::Snips, _) => "a scene shows a snip its commit's bundle doesn't hold, or home fetches no snips (no SNIPS_GITHUB_TOKEN), and a scene is never a blank frame",
			(Kind::ClipFrames, _) => "a scene shows a clip frame home doesn't hold, and a scene is never a blank frame",
			(Kind::Pages, Why::Gone) => "a scene shows a page its capture run's bundle doesn't give (no such Public Pages Capture run, one that didn't succeed, its artifact expired, its bundle didn't check out, or it holds that file of another address or moment): a page can't be taken again the same, so dispatch Public Pages Capture again and name the new run's pictures",
			(Kind::Pages, _) => "a scene shows a page home fetches no pages for (no SNIPS_GITHUB_TOKEN), or names it by no run or file a capture makes, and a scene is never a blank frame",
			(Kind::Walks, Why::Gone) => "a scene shows a walk its Snips Capture run doesn't give (no such successful run, its walks bundle expired or didn't check out, or the run left that app's walk out or has fewer steps): dispatch Snips Capture again and name the new run's walk",
			(Kind::Walks, _) => "a scene shows a walk home fetches no walks for (no SNIPS_GITHUB_TOKEN), or names it by no run, app or steps a capture makes, and a scene is never a blank frame",
		}
	}
}

/// What home holds of every item of one kind an essay names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handed {
	/// Each file of `<key>.<kind>/`.
	Ready(Vec<File>),
	/// Items not ready yet, by id: what the essay waits on of this kind.
	Waiting(Vec<String>),
	/// Never to be given as home holds them now.
	Refused(Refusal),
}

/// Why a kind could not be read: storage, or an item home holds that does
/// not read back (that essay fails the pass; the others are still given).
#[derive(Debug)]
pub enum Failed {
	Storage(sqlx::Error),
	Unreadable,
}

impl From<sqlx::Error> for Failed {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

/// Where home reads what it hands, and whose takes.
#[derive(Debug, Clone)]
pub struct Sources {
	/// The mailbox's subject (`mailbox::pass_subject`), whose takes an essay
	/// plays; with none, an essay naming a take waits.
	pub subject: Option<String>,
	/// The clips' frames (`clips::FRAMES_FOLDER`).
	pub frames: PathBuf,
	/// The bundles home fetches for waiting essays ([`crate::snips::FOLDER`]),
	/// or `None` when it fetches none (no `SNIPS_GITHUB_TOKEN`).
	pub snips: Option<PathBuf>,
	/// The public pages bundles home fetches for waiting essays
	/// ([`crate::snips::PAGES_FOLDER`]), or `None`, as [`Self::snips`].
	pub pages: Option<PathBuf>,
	/// The walks bundles home fetches for waiting essays
	/// ([`crate::snips::WALKS_FOLDER`]), or `None`, as [`Self::snips`].
	pub walks: Option<PathBuf>,
}

/// The `of` of `shot`'s `screen` picture, when it is one `from` `source`.
fn screen_of<'a>(shot: &'a Value, source: &str) -> Option<&'a Value> {
	let picture = shot.get("picture").filter(|picture| picture.get("kind").and_then(Value::as_str) == Some("screen"))?;
	picture.get("of").filter(|of| of.get("from").and_then(Value::as_str) == Some(source))
}

/// Every item of `kind` the cut list `essay` names, each once, in the order
/// first named. None for an essay that names none, or does not read.
#[must_use]
pub fn named(kind: Kind, essay: &Value) -> Vec<String> {
	let mut items: Vec<String> = Vec::new();
	let shots = essay.get("shots").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
	for item in shots.iter().flat_map(|shot| match kind {
		Kind::Takes => takes::named(shot),
		Kind::Snips => snips::named(shot),
		Kind::ClipFrames => clip_frames::named(shot),
		Kind::Pages => pages::named(shot),
		Kind::Walks => walks::named(shot),
	}) {
		if !items.contains(&item) {
			items.push(item);
		}
	}
	items
}

/// What home holds of `items`, of `kind`, as the module doc says.
///
/// # Errors
/// [`Failed`].
pub async fn hand(conn: &mut SqliteConnection, sources: &Sources, kind: Kind, items: &[String]) -> Result<Handed, Failed> {
	let (mut files, mut recorded) = (Vec::new(), Vec::new());
	let (mut missing, mut unheard, mut gone, mut waiting) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
	// Each by its id, once: two of a page's items may share one.
	let push = |items: &mut Vec<String>, item: &str| {
		let id = kind.id_of(item);
		if !items.contains(&id) {
			items.push(id);
		}
	};
	for item in items {
		let held = match kind {
			Kind::Takes => takes::hold(conn, sources, item).await?,
			Kind::Snips => snips::hold(conn, sources, item).await?,
			Kind::ClipFrames => clip_frames::hold(conn, sources, item).await?,
			Kind::Pages => pages::hold(conn, sources, item).await?,
			Kind::Walks => walks::hold(conn, sources, item).await?,
		};
		match held {
			Held::Files(handed) => files.extend(handed),
			Held::Record { bundle, record, pictures } => recorded.push((bundle, record, pictures)),
			Held::Waiting => push(&mut waiting, item),
			Held::Refused(Why::Missing) => push(&mut missing, item),
			Held::Refused(Why::Unheard) => push(&mut unheard, item),
			Held::Refused(Why::Gone) => push(&mut gone, item),
		}
	}
	// The kind's record of what it gives, or what of it disagrees (a page
	// two runs give unlike): those are gone.
	let given = match kind {
		Kind::Pages => pages::given(recorded).map_err(|items| items.iter().for_each(|item| push(&mut gone, item))).ok(),
		Kind::Walks => Some(walks::given(recorded)),
		Kind::Takes | Kind::Snips | Kind::ClipFrames => Some(snips::given(recorded)),
	};
	let refusal = |why, items| Handed::Refused(Refusal { kind, why, items });
	Ok(if !missing.is_empty() {
		refusal(Why::Missing, missing)
	} else if !unheard.is_empty() {
		refusal(Why::Unheard, unheard)
	} else if !gone.is_empty() {
		refusal(Why::Gone, gone)
	} else if !waiting.is_empty() {
		Handed::Waiting(waiting)
	} else {
		// Its record first, then its pictures.
		files.splice(0..0, given.unwrap_or_default());
		Handed::Ready(files)
	})
}

/// Every reason a refusal is said by ([`Refusal::reason`]), each once: what
/// reads a `.refused` as home's own (`renders_again`).
pub const REASONS: [&str; 9] = [
	"take-missing",
	"take-unheard",
	"snip-missing",
	"snip-gone",
	"clip-missing",
	"page-missing",
	"page-gone",
	"walk-missing",
	"walk-gone",
];

/// What home holds of everything a cut list names, kind by kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Given {
	/// Each kind it names, with its folder's files; empty for an essay that
	/// names nothing.
	Ready(Vec<(Kind, Vec<File>)>),
	/// Not ready yet: for each kind with an item not ready (a take not heard
	/// yet, a snip not fetched yet), in [`Kind::ALL`]'s order, those items by
	/// id. What the essay waits on (`crate::stage::Waiter::CardsEssays`).
	Waiting(Vec<(Kind, Vec<String>)>),
	Refused(Refusal),
}

/// Every kind `essay` names, handed, as the module doc says: the first
/// refusal in [`Kind::ALL`]'s order, else a wait, else every folder.
///
/// # Errors
/// [`Failed`].
pub async fn for_essay(conn: &mut SqliteConnection, sources: &Sources, essay: &Value) -> Result<Given, Failed> {
	let mut ready = Vec::new();
	let mut waiting = Vec::new();
	for kind in Kind::ALL {
		let items = named(kind, essay);
		if items.is_empty() {
			continue;
		}
		match hand(conn, sources, kind, &items).await? {
			Handed::Ready(files) => ready.push((kind, files)),
			Handed::Waiting(items) => waiting.push((kind, items)),
			Handed::Refused(refusal) => return Ok(Given::Refused(refusal)),
		}
	}
	Ok(if waiting.is_empty() { Given::Ready(ready) } else { Given::Waiting(waiting) })
}

/// Whether home still holds every item of `kind` the cut list `essay` names.
/// Held but not read back is never a reason to take an essay back.
///
/// # Errors
/// A storage failure.
pub async fn still_held(conn: &mut SqliteConnection, sources: &Sources, kind: Kind, essay: &Value) -> Result<bool, sqlx::Error> {
	for item in named(kind, essay) {
		let held = match kind {
			Kind::Takes => takes::still_held(conn, sources, &item).await?,
			Kind::Snips => snips::still_held(conn, sources, &item).await?,
			Kind::ClipFrames => clip_frames::still_held(conn, sources, &item).await?,
			Kind::Pages => pages::still_held(conn, sources, &item).await?,
			Kind::Walks => walks::still_held(conn, sources, &item).await?,
		};
		if !held {
			return Ok(false);
		}
	}
	Ok(true)
}

/// `<key>.<kind>`, a kind's folder in `folder`.
#[must_use]
pub fn folder_of(folder: &Path, key: &str, kind: Kind) -> PathBuf {
	folder.join(String::from(key) + "." + kind.folder())
}

/// `.<key>.<kind>.partial`, where [`write`] makes it.
#[must_use]
pub fn partial_of(folder: &Path, key: &str, kind: Kind) -> PathBuf {
	folder.join(String::from(".") + key + "." + kind.folder() + ".partial")
}

/// Which kind's folder `name` is, and its essay's key, when it is one.
#[must_use]
pub fn parse_folder(name: &str) -> Option<(&str, Kind)> {
	Kind::ALL
		.into_iter()
		.find_map(|kind| name.strip_suffix(kind.folder())?.strip_suffix('.').map(|key| (key, kind)))
}

/// Whether `name` is a partial [`write`] leaves when stopped.
#[must_use]
pub fn is_partial(name: &str) -> bool {
	name.starts_with('.') && Kind::ALL.into_iter().any(|kind| name.ends_with(&(String::from(".") + kind.folder() + ".partial")))
}

/// Whether `name` is a relative path of plain parts, so a file is written
/// only inside its folder.
fn plain(name: &str) -> bool {
	!name.is_empty() && Path::new(name).components().all(|part| matches!(part, Component::Normal(_)))
}

/// Writes `files` into `<key>.<kind>/` in `folder`, whole.
///
/// It is made in a temporary folder and renamed into place, replacing what
/// a stopped pass left. A name that is not a plain relative path is an
/// error, and nothing is put in place.
///
/// # Errors
/// An I/O failure, or a name that is not plain.
pub async fn write(folder: &Path, key: &str, kind: Kind, files: &[File]) -> std::io::Result<()> {
	if let Some((name, _)) = files.iter().find(|(name, _)| !plain(name)) {
		return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, String::from("not a plain name: ") + name));
	}
	let partial = partial_of(folder, key, kind);
	let into = folder_of(folder, key, kind);
	for left in [&partial, &into] {
		remove_dir(left).await?;
	}
	tokio::fs::create_dir_all(&partial).await?;
	for (name, body) in files {
		let to = partial.join(name);
		if let Some(parent) = to.parent() {
			tokio::fs::create_dir_all(parent).await?;
		}
		tokio::fs::write(to, body).await?;
	}
	tokio::fs::rename(&partial, &into).await
}

/// Removes `dir` and all in it; one already gone is no error.
///
/// # Errors
/// An I/O failure.
pub async fn remove_dir(dir: &Path) -> std::io::Result<()> {
	match tokio::fs::remove_dir_all(dir).await {
		Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
		_ => Ok(()),
	}
}

/// A file's bytes from home's drive, or `None` when it is not there. A
/// file there that does not read is [`Failed::Unreadable`].
async fn read_held(path: &Path) -> Result<Option<Vec<u8>>, Failed> {
	match tokio::fs::read(path).await {
		Ok(bytes) => Ok(Some(bytes)),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
		Err(_) => Err(Failed::Unreadable),
	}
}

/// The lowercase hex SHA-256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
	use sha2::{Digest, Sha256};
	hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn each_kind_has_its_own_folder_and_reason() {
		let folders: Vec<_> = Kind::ALL.iter().map(|kind| kind.folder()).collect();
		assert_eq!(folders, ["takes", "snips", "clips", "pages", "walks"]);
		for kind in Kind::ALL {
			assert_eq!(parse_folder(&(String::from("hello-2026-11-20.") + kind.folder())), Some(("hello-2026-11-20", kind)));
			assert!(is_partial(&(String::from(".hello-2026-11-20.") + kind.folder() + ".partial")));
		}
		assert_eq!(parse_folder("hello-2026-11-20.json"), None);
		assert_eq!(parse_folder("hello-2026-11-20takes"), None);
		let reasons: Vec<_> = [
			(Kind::Takes, Why::Missing),
			(Kind::Takes, Why::Unheard),
			(Kind::Snips, Why::Missing),
			(Kind::Snips, Why::Gone),
			(Kind::ClipFrames, Why::Missing),
			(Kind::Pages, Why::Missing),
			(Kind::Pages, Why::Gone),
			(Kind::Walks, Why::Missing),
			(Kind::Walks, Why::Gone),
		]
		.into_iter()
		.map(|(kind, why)| Refusal { kind, why, items: Vec::new() }.reason())
		.collect();
		assert_eq!(
			reasons,
			[
				"take-missing",
				"take-unheard",
				"snip-missing",
				"snip-gone",
				"clip-missing",
				"page-missing",
				"page-gone",
				"walk-missing",
				"walk-gone"
			]
		);
		assert_eq!(reasons, REASONS, "every reason, each in REASONS");
		for kind in Kind::ALL {
			for why in [Why::Missing, Why::Unheard, Why::Gone] {
				assert!(REASONS.contains(&Refusal { kind, why, items: Vec::new() }.reason()), "{kind:?} {why:?}");
			}
		}
	}

	#[tokio::test]
	async fn a_folder_is_written_whole_and_only_inside_itself() {
		let dir = tempfile::tempdir().unwrap();
		let files = vec![
			(String::from("snips.json"), b"{}".to_vec()),
			(String::from("web/phone/a-0123456789ab.png"), b"png".to_vec()),
		];
		write(dir.path(), "k-2026-11-20", Kind::Snips, &files).await.unwrap();
		let into = dir.path().join("k-2026-11-20.snips");
		assert_eq!(std::fs::read(into.join("web/phone/a-0123456789ab.png")).unwrap(), b"png");
		assert!(!partial_of(dir.path(), "k-2026-11-20", Kind::Snips).exists());
		for bad in ["../escape.png", "/abs.png", "a/../../b.png", ""] {
			let files = vec![(String::from(bad), b"x".to_vec())];
			assert!(write(dir.path(), "k-2026-11-20", Kind::Snips, &files).await.is_err(), "{bad:?}");
		}
		assert!(into.join("snips.json").exists(), "a refused write leaves what was given alone");
	}

	#[test]
	fn a_cut_list_names_each_item_once_by_kind() {
		let essay = serde_json::json!({ "shots": [
			{ "picture": { "kind": "screen", "of": { "from": "snips", "id": "web:/settings:phone:1.2.3:0123456789ab", "commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "at": [] } } },
			{ "picture": { "kind": "screen", "of": { "from": "snips", "id": "web:/settings:phone:1.2.3:0123456789ab", "commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" } } },
			{ "picture": { "kind": "screen", "of": { "from": "snips", "id": "web:/settings:phone:1.2.3:0123456789ab", "commit": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" } } },
			{ "picture": { "kind": "screen", "of": { "from": "clips", "clip": "studio-promo", "side": "after", "file": "01.png" } } },
			{ "picture": { "kind": "snip", "id": "web:/old:phone:1.2.3:0123456789ab" } },
			{ "picture": { "kind": "still", "source": "a.png" } },
		]});
		assert_eq!(
			named(Kind::Snips, &essay),
			[
				"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa web:/settings:phone:1.2.3:0123456789ab",
				"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb web:/settings:phone:1.2.3:0123456789ab"
			],
			"a snip by its commit and id: the same id of another commit is another"
		);
		assert_eq!(named(Kind::ClipFrames, &essay), ["studio-promo/after/01.png"]);
		assert!(named(Kind::Takes, &essay).is_empty());
		assert!(named(Kind::Snips, &serde_json::json!("not a cut list")).is_empty());
	}
}
