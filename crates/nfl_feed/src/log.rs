//! The play log, as a fold over summaries: what each new response adds.
//!
//! The log is append-only. A play home has not seen is a new entry; a play
//! whose fields changed since the latest entry for it (ESPN revises plays:
//! a spot, a penalty, a scoring change) is a new entry that supersedes that
//! one; a play seen again unchanged adds nothing. A play ESPN stops listing
//! is not recorded as gone: the raw responses still show it.
//!
//! [`changes`] is the one rule, shared by the store (`game_feed_repo`, over
//! its rows) and by [`PlayLog`] (in memory, for replay), so replaying the kept
//! responses through [`PlayLog`] gives the log the store holds (GW10).

use crate::summary::Play;
use std::collections::HashMap;

/// One play a response adds to the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change<'a, R> {
	pub play: &'a Play,
	/// The entry this one revises, or `None` for a play not seen before.
	pub supersedes: Option<R>,
}

/// What `plays` add to a log whose latest entry per play id `latest` gives
/// (its handle and the play as that entry holds it), in `plays`' order.
pub fn changes<'a, 'l, R: Copy>(plays: &'a [Play], latest: impl Fn(&str) -> Option<(R, &'l Play)>) -> Vec<Change<'a, R>> {
	plays
		.iter()
		.filter_map(|play| match latest(&play.id) {
			None => Some(Change { play, supersedes: None }),
			Some((_, held)) if held == play => None,
			Some((handle, _)) => Some(Change { play, supersedes: Some(handle) }),
		})
		.collect()
}

/// One entry of the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
	pub game_id: String,
	pub play: Play,
	/// The kept response it was first read from.
	pub response_id: i64,
	/// When home first saw this version: that response's `fetched_at`.
	pub seen_at: String,
	/// The position in the log of the entry this one revises.
	pub supersedes: Option<usize>,
}

/// The log in memory: entries in the order they were added.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayLog {
	entries: Vec<Entry>,
	latest: HashMap<(String, String), usize>,
}

impl PlayLog {
	/// Fold one summary's plays for `game_id`, read from response
	/// `response_id` fetched at `fetched_at`. Returns how many entries it
	/// added.
	pub fn apply(&mut self, game_id: &str, response_id: i64, fetched_at: &str, plays: &[Play]) -> usize {
		let added: Vec<(Play, Option<usize>)> = changes(plays, |id| self.latest.get(&(game_id.to_owned(), id.to_owned())).map(|at| (*at, &self.entries[*at].play)))
			.into_iter()
			.map(|change| (change.play.clone(), change.supersedes))
			.collect();
		let count = added.len();
		for (play, supersedes) in added {
			self.latest.insert((game_id.to_owned(), play.id.clone()), self.entries.len());
			self.entries.push(Entry {
				game_id: game_id.to_owned(),
				play,
				response_id,
				seen_at: fetched_at.to_owned(),
				supersedes,
			});
		}
		count
	}

	#[must_use]
	pub fn entries(&self) -> &[Entry] {
		&self.entries
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::summary;

	const FINAL: &[u8] = include_bytes!("../testdata/summary-401873010-final.json");

	#[test]
	fn a_play_is_added_once_and_a_revision_supersedes_it() {
		let plays = summary::parse(FINAL).unwrap().plays;
		let mut log = PlayLog::default();
		assert_eq!(log.apply("401873010", 1, "t1", &plays[..100]), 100);
		assert_eq!(log.apply("401873010", 2, "t2", &plays), 80, "the first hundred are already held");
		assert_eq!(log.apply("401873010", 3, "t3", &plays), 0, "nothing changed");

		let mut revised = plays.clone();
		revised[5].yard_line = revised[5].yard_line.map(|y| y + 1);
		assert_eq!(log.apply("401873010", 4, "t4", &revised), 1);
		let last = log.entries().last().unwrap();
		assert_eq!((last.supersedes, last.response_id, last.seen_at.as_str()), (Some(5), 4, "t4"));
		assert_eq!(log.apply("401873010", 5, "t5", &revised), 0, "the revision is now the latest");
		// Back to what it was is a change from the latest, so a third entry.
		assert_eq!(log.apply("401873010", 6, "t6", &plays), 1);
		assert_eq!(log.entries().last().unwrap().supersedes, Some(180));

		// The same play id in another game is another play.
		assert_eq!(log.apply("other", 7, "t7", &plays[..1]), 1);
		assert_eq!(log.entries().last().unwrap().supersedes, None);
	}
}
