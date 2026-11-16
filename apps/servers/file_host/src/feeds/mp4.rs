//! A rendered video's length, as its own mp4 says it (server#906).
//!
//! The recorder writes an mp4 whose movie header (`moov` › `mvhd`) carries
//! the movie's duration and the timescale it is counted in. That header is
//! what any player reads for the length it shows, so it is the honest
//! source, and reading it costs a few small reads per video: the top-level
//! boxes' headers (each seeked past, `mdat` included, never read) and the
//! first header inside `moov`. No probe is run and nothing is stored, so a
//! video rendered before this read existed has a length too, and one whose
//! video was let go has none.
//!
//! **Bounded.** At most [`MOST_BOXES`] boxes are looked at on each level,
//! and nothing larger than a box header and `mvhd`'s first 32 bytes is ever
//! read, so a file that is not an mp4, or a broken one, costs the same
//! handful of reads and answers `None`.

use std::path::Path;
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};

/// The most boxes [`millis`] looks at on one level before it gives up: a
/// recorder's mp4 has three or four at the top (`ftyp`, `moov`, `mdat`,
/// maybe `free`) and `mvhd` first inside `moov`.
pub const MOST_BOXES: usize = 16;

/// The video at `path`'s length in milliseconds, as its movie header says
/// it: `None` for a file that isn't there or isn't an mp4 whose header says
/// a length (a fragmented one's is 0; an unknown one's all ones).
pub async fn millis(path: &Path) -> Option<u64> {
	let mut file = tokio::fs::File::open(path).await.ok()?;
	let end = file.metadata().await.ok()?.len();
	let (moov_body, moov_end) = find(&mut file, 0, end, *b"moov").await?;
	let (mvhd_body, mvhd_end) = find(&mut file, moov_body, moov_end, *b"mvhd").await?;
	file.seek(std::io::SeekFrom::Start(mvhd_body)).await.ok()?;
	let mut head = [0_u8; 32];
	let len = usize::try_from(mvhd_end.checked_sub(mvhd_body)?).ok()?.min(head.len());
	file.read_exact(head.get_mut(..len)?).await.ok()?;
	header_millis(head.get(..len)?)
}

/// The body's start and the box's end of the first box of `kind` among the
/// boxes from `start` to `end`, looking at no more than [`MOST_BOXES`].
async fn find(file: &mut tokio::fs::File, start: u64, end: u64, kind: [u8; 4]) -> Option<(u64, u64)> {
	let mut at = start;
	for _ in 0..MOST_BOXES {
		if end.checked_sub(at)? < 8 {
			return None;
		}
		file.seek(std::io::SeekFrom::Start(at)).await.ok()?;
		let mut header = [0_u8; 8];
		file.read_exact(&mut header).await.ok()?;
		let (size, name) = header.split_at(4);
		let size = u64::from(u32::from_be_bytes(size.try_into().ok()?));
		let (body, box_end) = match size {
			// To the end of the file.
			0 => (at + 8, end),
			// A 64-bit size follows the name.
			1 => {
				let mut large = [0_u8; 8];
				file.read_exact(&mut large).await.ok()?;
				(at + 16, at.checked_add(u64::from_be_bytes(large))?)
			}
			size => (at + 8, at.checked_add(size)?),
		};
		if box_end > end || box_end < body {
			return None;
		}
		if name == kind {
			return Some((body, box_end));
		}
		if box_end == at {
			return None;
		}
		at = box_end;
	}
	None
}

/// The length in milliseconds a movie header's body (`mvhd`, from its
/// version byte) says, if it says one.
fn header_millis(body: &[u8]) -> Option<u64> {
	let (&version, rest) = body.split_first()?;
	// Three bytes of flags, then the version's times.
	let rest = rest.get(3..)?;
	let (timescale, duration) = match version {
		0 => {
			let scale = u32::from_be_bytes(rest.get(8..12)?.try_into().ok()?);
			let duration = u32::from_be_bytes(rest.get(12..16)?.try_into().ok()?);
			(scale, (duration != u32::MAX).then_some(u64::from(duration))?)
		}
		1 => {
			let scale = u32::from_be_bytes(rest.get(16..20)?.try_into().ok()?);
			let duration = u64::from_be_bytes(rest.get(20..28)?.try_into().ok()?);
			(scale, (duration != u64::MAX).then_some(duration)?)
		}
		_ => return None,
	};
	if timescale == 0 || duration == 0 {
		return None;
	}
	let millis = u128::from(duration) * 1_000 / u128::from(timescale);
	u64::try_from(millis).ok()
}

#[cfg(test)]
pub(crate) mod tests {
	use super::*;

	/// A box of `kind` around `body`.
	fn boxed(kind: [u8; 4], body: &[u8]) -> Vec<u8> {
		let mut bytes = u32::try_from(body.len() + 8).unwrap().to_be_bytes().to_vec();
		bytes.extend_from_slice(&kind);
		bytes.extend_from_slice(body);
		bytes
	}

	/// An mp4's boxes as a recorder lays them out: `ftyp`, then `moov`
	/// holding an `mvhd` of `version` saying `duration` in `timescale`, then
	/// `mdat`; or `mdat` before `moov` when `moov_last`.
	pub(crate) fn mp4(version: u8, timescale: u32, duration: u64, moov_last: bool) -> Vec<u8> {
		let mut mvhd = vec![version, 0, 0, 0];
		if version == 0 {
			mvhd.extend_from_slice(&[0; 8]);
			mvhd.extend_from_slice(&timescale.to_be_bytes());
			mvhd.extend_from_slice(&u32::try_from(duration).unwrap().to_be_bytes());
		} else {
			mvhd.extend_from_slice(&[0; 16]);
			mvhd.extend_from_slice(&timescale.to_be_bytes());
			mvhd.extend_from_slice(&duration.to_be_bytes());
		}
		mvhd.extend_from_slice(&[0; 80]);
		let mut moov = boxed(*b"mvhd", &mvhd);
		moov.extend(boxed(*b"trak", &[0; 40]));
		let moov = boxed(*b"moov", &moov);
		let mdat = boxed(*b"mdat", &[7; 4_000]);
		let mut file = boxed(*b"ftyp", b"isom\0\0\x02\0isomiso2avc1mp41");
		if moov_last {
			file.extend(mdat);
			file.extend(moov);
		} else {
			file.extend(moov);
			file.extend(mdat);
		}
		file
	}

	async fn read(bytes: &[u8]) -> Option<u64> {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("call.mp4");
		tokio::fs::write(&path, bytes).await.unwrap();
		millis(&path).await
	}

	#[tokio::test]
	async fn a_videos_length_is_its_movie_headers_wherever_moov_is() {
		assert_eq!(read(&mp4(0, 1_000, 754_321, false)).await, Some(754_321), "moov first (faststart)");
		assert_eq!(read(&mp4(0, 90_000, 90_000 * 61, true)).await, Some(61_000), "moov after mdat");
		assert_eq!(read(&mp4(1, 600, 600 * 3_600, false)).await, Some(3_600_000), "a version 1 header");
	}

	#[tokio::test]
	async fn anything_that_says_no_length_is_none() {
		assert_eq!(read(&mp4(0, 1_000, 0, false)).await, None, "a fragmented mp4's 0");
		assert_eq!(read(&mp4(0, 1_000, u64::from(u32::MAX), false)).await, None, "unknown");
		assert_eq!(read(&mp4(0, 0, 5_000, false)).await, None, "no timescale");
		assert_eq!(read(b"not an mp4 at all, only text").await, None);
		assert_eq!(read(b"").await, None);
		let mut cut = mp4(0, 1_000, 5_000, true);
		cut.truncate(cut.len() - 60);
		assert_eq!(read(&cut).await, None, "a moov cut short");
		assert_eq!(millis(Path::new("/nowhere/call.mp4")).await, None);
		// More boxes than it looks at before moov.
		let mut many = Vec::new();
		for _ in 0..MOST_BOXES {
			many.extend(boxed(*b"free", &[0; 4]));
		}
		many.extend(mp4(0, 1_000, 5_000, false));
		assert_eq!(read(&many).await, None);
	}
}
