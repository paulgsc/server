# The phrase shop's home half

KPLAY's KP4 (paulgsc/some-ui#2853, epic paulgsc/some-ui#2849; this repo's
paulgsc/server#1261). The owner says, in English, what they want to say. Home
finds the Korean phrases that do it, deterministically, and a model writes a
phrase only when nothing fits (KP5), from the hunches home kept. The phone's
half is some-ui's `packages/ui/topik/src/lib/topik/shop/` (a pure machine, and
a runtime over a port to home). This doc is home's.

## What home does

- **The intents are in the meaning index.** Every phrase on some-ui's shelf
  (`packages/ui/topik/src/lib/topik/phrases/shelf.ts`) has a stable `ph.*` id
  and English `intents`. Home embeds each intent once, at start, with the
  meaning index's model (`file_host::meaning`, Model2Vec's
  `potion-multilingual-128M`, the one the take library uses): one vector per
  intent, keyed by phrase id, in memory (`file_host::phrases::Phrases`).
- **How the intents reach home: a synced file** (call C1605).
  `apps/servers/file_host/src/phrases/intents.json` is a copy of the shelf's
  ids and intents, in `some-ui.phrase-intents/1`, with the shelf commit it
  was copied from in `source`. It is compiled in, so a phrase reaches home
  with a release. A copy that does not read (a phrase named twice, one with
  no intent, an intent the model knows none of the words of) takes down the
  lookup alone: it answers `503` (the phone reads it as unreached and keeps
  the hunch to send again), the take library's search runs on, and the
  failure is held open as `failures_open{process="home-transcription",
  step="phrase_index"}`, which `HomeFailuresOpen` tells the owner of, until
  a release with a shelf that reads (`meaning::phrase_index`). Each side tolerates the other
  being ahead: the phone drops a phrase id its build does not hold, and home
  names only phrases in its copy.
- **The lookup:** `POST /api/v1/phrases/lookup`, a paired phone's, body
  `{ words, hunch }`, answers the shop port's `Lookup`:
  `{ kind: "hit", phrases: [id, …] }` closest first, or `{ kind: "kept" }`.
  `hunch` is the phone's name for the hunch (its runtime's id and the hunch's
  number this session), the same each time it is sent.
- **A miss is kept in the answer.** On a miss the request keeps the hunch
  (`phrase_hunch`, `korean_repo::hunch`) before it answers `kept`, so a miss
  is never lost between two calls, and a hunch sent again under its name is
  kept once. A hit keeps nothing.
- **Home looks up a hunch it hears, too.** The transcriber's pass looks
  up the words it heard in a `hunch` note, in the transaction that keeps the
  hearing, and keeps a miss under the note's `ref` the same way
  (`transcribe::pass`), so a hunch whose phone left mid-take, could not
  reach home, or stopped waiting is still kept. The phone's own lookup of
  the same hunch, before or after, changes nothing. `asked_at` is when the
  owner asked: when home answered, for the phone's lookup, and when the
  phone recorded the take, for home's hearing, however long the hearing
  waited (a backlog, or a take heard again). Without the shelf's
  index (the meaning index off, or a shelf that did not index), a heard hunch
  is kept as unmatched: it is heard once, so nothing would look it up later,
  and the record comes before precision. Heard words are trimmed
  and cut to the 200 characters a hunch holds. A storage failure rolls the
  hearing back with it, and the next pass hears the note again. It adds no
  clock, lane or bound to the pass (docs/home-compute.md): one read by the
  note's key and a scan of the shelf's few dozen intents.
- **Hearing a spoken hunch uses the routes every voice note uses.** A spoken
  hunch is a voice note of kind `hunch`, its `ref` the hunch's name:
  `PUT /voice-notes/:id?of=hunch&ref=<hunch>`, `POST
  /voice-notes/:id/fast-track` to put it first in the transcriber's line,
  then `GET /voice-notes/:id/heard` for the words. `hunch` notes are heard in
  English (`voice_repo::verdict::expected_language`), and no route takes a
  target phrase: hearing never cheats. They are not filed for the mailbox
  (that list is by hand, and `hunch` is not on it), so a hunch's words stay
  on home. My data plays one back, as it plays every kind (`voice_repo::PLAYED_BACK`),
  and deletes one only when home could not hear it, as for every kind.

## The threshold

A phrase's score is its nearest intent's cosine with the hunch's words. Every
phrase scoring at least `THRESHOLD` (0.75) and within `NEAR` (0.1) of the best
is a hit. Both are policy (`file_host::phrases`, call C1606), set from these
scores of the real model over the seed shelf (14 phrases), measured
2026-11-27 (best phrase first, then the runner-up):

| Hunch | Best | Runner-up |
|---|---|---|
| where's my phone | 1.000 where-is-my-phone | 0.254 |
| I lost my phone | 0.772 where-is-my-phone | 0.329 |
| take a seat | 0.809 please-sit-here | 0.206 |
| it's freezing, let's head inside | 0.765 cold-so-shall-we-go-in | 0.314 |
| no idea | 0.844 i-didnt-know | 0.224 |
| it was expensive but worth it | 0.821 pricey-but-good | 0.305 |
| I'm going to get something to eat | 0.932 going-to-eat | 0.806 shall-we-eat-together |
| is it supposed to rain tomorrow | 0.919 will-it-rain-tomorrow | 0.527 |
| do you want to grab lunch together | 0.497 shall-we-eat-together | 0.415 |
| I'm swamped today | 0.454 will-it-rain-tomorrow | 0.417 bit-busy-today |
| see you tomorrow | 0.746 will-it-rain-tomorrow | 0.418 |
| call me later | 0.615 where-is-my-phone | 0.269 |
| this food is amazing | 0.628 did-you-eat | 0.618 |
| how do I get to the train station | 0.330 going-to-eat | 0.265 |

A static model is coarse: an unrelated sentence that shares an intent's words
reaches 0.75 ("see you tomorrow"), and some honest paraphrases score below
0.5 ("grab lunch together"). The threshold errs toward a miss, because a miss
costs little (it is kept, and KP5 writes the phrase) and a wrong phrase is
shown to the owner as one that does what they meant. `NEAR` keeps "eat
together" (0.806) off "I'm going to get something to eat" (0.932). If the
misses prove too many, the move is a better model under a new name, as the
take library's doc says, and a new call for the numbers.

## Invariants

1. **Home decides, and holds the numbers.** The phone never sends a
   threshold or a target phrase, and the route takes no field but `words` and
   `hunch` (an unknown field is a `422`).
2. **A miss is kept before `kept` is answered, once per hunch,** and a
   heard hunch's miss in its hearing's transaction. *Held by*
   `handlers::phrases`' tests, `transcribe::pass`' and `korean_repo::hunch`'s.
3. **The shelf on home is the synced file, and only it.** No route or table
   writes a phrase's intents; a new phrase is a new copy of the file in a
   release.
4. **A hunch is the owner's words** (docs/identity.md invariant 41):
   subject-scoped, append-only, never logged, and gone only with the account.
5. **Hearing a hunch is hearing a voice note.** No route hears on a request's
   path (docs/home-compute.md; `file_host::transcribe`), and none is told a
   phrase.

## What is not here yet

- **A check that the copy matches the shelf.** The shelf is not on some-ui's
  `main` yet, so nothing compares the two; a follow-up issue holds it.
- **KP5**, the routine that reads the kept hunches and writes phrases.
