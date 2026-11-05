# The take library

The owner's decision 8 of 2026-11-04 (paulgsc/some-ui#2248, Studio): takes
are a library on home, separate from scenes. Each take has an ID and is
transcribed, kept and played from home; the library is searchable by meaning
("the take where I talked about the brand"); and a take is attached to a scene
by its ID, so it can be recorded before the scene exists. The story is
paulgsc/some-ui#2590 (ST32). This doc is its server half.

## What home already does, and what this adds

A take is a voice note said for `studio` (docs/identity.md, invariant 23): a
scene's take (`<project>:<shot>:<take>`), an idea's spoken answer
(`idea:<idea>:<question>`), or a take recorded with no scene, whose `ref` the
phone names as it likes within the rule every `ref` meets. Home keeps each
one, hears it with the transcriber (`file_host::transcribe`) and keeps every
hearing (`voice_repo::transcript`).

This adds:

- **The library read**, `GET /api/v1/voice-notes/takes[?q=]`
  (`voice_repo::takes`): every take the caller keeps, newest first, at most
  50, each with the words `GET /voice-notes/:id/heard` would answer, how many
  there are, and with `q` the takes whose words hold every term of it, how
  many it found, and `by`, which says how the takes were chosen (`newest`,
  `words`, or `meaning`, by the index below).
- **Playback from home**: `GET /api/v1/voice-notes/:id/audio` answers a
  `studio` take as it answers a Rust Rounds note (`voice_repo::PLAYED_BACK`).
  The phone lets a take's audio go once home holds it (HOME-FIRST D1), so
  without this a take could be heard only before then. This is also ST24's
  open question (paulgsc/some-ui#2322, "where a take's audio plays from"),
  answered by the same decision: from home.

## Where the meaning index lives

ST32's first design call. **Decided here** (this doc, and the PR that adds
it): the index lives on home, in file_host's SQLite, beside the transcripts it
is made from, and it is made on home's CPU by a model home holds. It is
built (paulgsc/server#675 part 2): `voice_repo::meaning` keeps and reads the
vectors, `file_host::meaning` holds the model, and the transcriber's pass
writes them. docs/identity.md invariant 35 is what it keeps.

**The shape:**

- **A derived table,** `voice_note_meaning`, one row per take and model: the
  subject, the note, the model's name, and the take's vector (256 `f32`s,
  little-endian, as a blob). It is made from the take's chosen words (the ones
  the library lists), so it can always be made again from the transcripts, and
  a better model's vectors go under a new model name beside the old, as a
  better transcript does. A better transcript of the same take replaces the
  take's vector under the same model, since the words it was made from are no
  longer the ones listed.
- **Written where the words are,** in the transcriber's pass, in the
  transaction that keeps a hearing, so a take is searchable by meaning as soon
  as it is heard and no second worker polls for it. A take heard before the
  index existed gets its vector from the same pass: each pass first gives at
  most `transcribe::pass::BATCH` (20) takes with heard words and no vector
  theirs, each in its own transaction under the deletion hold, so the
  catch-up is one-time in effect and bounded per pass as the transcriber's
  hearing is (`voice_repo::meaning::unindexed`).
- **A static embedding model:** Model2Vec's `potion-multilingual-128M` (MIT,
  256 dimensions, about 512 MB on disk, multilingual, so a take that mixes
  Korean and English is one vector). Its vector for a text is a lookup of each
  token's row and their mean, so making one costs microseconds and holds no
  thread: the query is embedded on the request's path, and the index is
  written inside the hearing's transaction without lengthening it. It is
  mounted from `/models` as whisper's model is (`file_host::meaning::DIR`,
  `/models/potion-multilingual-128M/`, with its `tokenizer.json`,
  `model.safetensors` and `config.json`; `nix/whisper-manager`'s `whisper
  fetch` downloads them, pinned to one commit and checked by SHA-256), and
  home downloads nothing at run time. Its tokenizer is bge-m3's (a
  SentencePiece unigram with a precompiled normalizer), which the `tokenizers`
  crate reads; that crate and `safetensors` are the dependencies it adds. A
  vector is Model2Vec's: the mean of the rows of the text's tokens, the
  unknown token left out, divided by its length (`config.json`'s
  `normalize`); a text whose every token is unknown is all zeros. The whole
  of a take's words is embedded, with no cut at 512 tokens.
- **A search is a scan,** the cosine of the query's vector with each of the
  subject's take vectors, best first (takes as near as each other newest
  first). A take is found when its vector points the query's way at all (a
  cosine above zero), and `matched` counts those. One owner's library is
  thousands of takes at 1 KiB each, so a scan reads a few MB, the same order
  as the words search reads now; no approximate index is needed until the
  library is a hundred times that.
- **Off unless switched on** (`MEANING_ENABLED`), as the transcriber is; on,
  the server refuses to start without the model, and without the
  transcriber (`TRANSCRIBE_ENABLED`), whose pass is what writes the index.
  Off, or while any take home heard words in has no vector yet (the catch-up
  has not reached it), or for asked words the model knows none of, a search
  is by words, and `by` says `words`: a search by meaning would miss the take
  with no vector, and a search by words does not.

**Why there, and not elsewhere:**

- **Not on the phone.** The phone lets a take go once home holds it, and
  decision 8 says the library is home's. A phone-side index would also need
  every take's words on the phone.
- **Not a hosted vector store** (Cloudflare Vectorize, a model API's
  embeddings). A take's words are what the owner said, personal data, which
  stays on the home network (some-ui CLAUDE.md, "Private only where the data
  is personal"); a third party would hold a copy of every take's meaning.
- **Not a transformer embedder.** whisper already holds about 500 MB and two
  threads while it hears. A transformer's query embedding would hold a thread
  per search, on the request's path, for a gain a library of one person's
  takes does not need. The static model's vectors are worse than a
  transformer's at fine meaning and good at "the take about the brand"; if
  that proves too coarse, a transformer model's vectors go under a new model
  name, as above, and nothing else changes.

**What the index must keep** (docs/identity.md invariant 35):

- A vector is made from what was said, so it is treated as what was said:
  subject-scoped, never logged, never filed for the mailbox, never answered by
  a route (the search answers takes, not vectors).
- It leaves with the take: My data's delete of a take
  (`voice_repo::delete`) removes its vectors in the same transaction, and the
  account sweep and the table's own trigger remove them with the account
  (invariants 9, 12 and 22).
- It holds nothing but the note's id, the model and the vector: no `ref`, no
  text, no time.

The study nudge's scaling invariants (docs/study-nudge.md) do not reach it:
it reads no signal and holds nothing per subject beyond the subject's own
takes. The identity doc's do, as above.

## Cost

Each library read is one statement: it walks the caller's note descriptions
(`voice_note` by its key, never the audio), keeps the `studio` ones, and reads
each one's chosen hearing by the transcripts' key, so no other kind's
transcript is read. It then keeps at most 50. It grows with how many notes the
owner keeps, never with other subjects', and only the owner's own paired phone
asks it, once per screen opened or search said. If the owner's other notes
ever make that walk slow, an index on `voice_note (subject_id, of_kind)` makes
it the takes alone.

A search by meaning adds a second statement, the subject's vectors under one
model by the index's key (1 KiB a heard take), and embeds the asked words (at
most 200 characters) on the request's path, a lookup per token. The
transcriber's catch-up costs one read a pass over the notes' descriptions, as
its own queue does, and at most 20 small writes; a take heard adds one write
to the transaction that keeps its hearing.
