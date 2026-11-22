-- Judges each take whose last attempt failed before home judged attempts
-- (paulgsc/server#913; HEARD1, #908, epic paulgsc/some-ui#3070).
--
-- From HEARD1 on, every final attempt at hearing a note is kept with its
-- verdict (`file_host::transcribe::verdict::judge`, VH4), and a Rust Rounds or
-- interview take whose attempts all failed is filed to the mailbox by that
-- `failed` verdict, so the week's routine stops waiting on it. A note whose
-- model used up its attempts before HEARD1 deployed has no verdict, is never
-- tried again (`voice_repo::transcript::waiting`) and so is never judged or
-- filed: its week waits on it for good.
--
-- This writes the verdict `judge` gives such an attempt, word for word: rule
-- `heard-1`, `failed`, the reason named by the outcome (`unreadable`,
-- `too_long`, `deadline`, or `engine` for `failed`), no signals. A failure is
-- judged on its outcome alone, so judging it now is judging it as it was. An
-- ending kept before HEARD1 is not judged here: its verdict needs the engine's
-- measures, which were never kept.
--
-- The attempt judged is a model's last on a note, a failure, numbered at
-- least 3: `file_host::transcribe::pass::ATTEMPTS`, past which the worker does
-- not try that model again. A take of every kind, with no list of kinds
-- (C976, the owner: "we never descend to the granular; we implement
-- solutions that target the family"): no clause here reads the kind, so
-- every kind is covered, and its test walks `voice_repo::Of::ALL`, which the
-- compiler keeps whole. One judged here is listed under Needs your attention (VH5) like
-- any take home couldn't hear, and a Rust Rounds or interview take is filed,
-- so its week stops waiting. It raises no alert: `TranscriptGarbled` counts
-- verdicts as the worker keeps them, not rows.
--
-- Each is dated at its attempt's own `heard_at`, when `judge` would have
-- judged it, so a note's newest verdict (`voice_repo::verdict::newest`, by
-- `judged_at`) stays the failure judged last, not whichever this wrote.
--
-- Append-only, as the table is: new rows only, none where the attempt already
-- has a verdict under `heard-1`.
INSERT INTO voice_transcript_verdict (subject_id, note_id, model, attempt, rule, verdict, reasons, judged_at)
SELECT t.subject_id, t.note_id, t.model, t.attempt, 'heard-1', 'failed',
       CASE t.outcome WHEN 'failed' THEN 'engine' ELSE t.outcome END,
       t.heard_at
FROM voice_transcript t
WHERE t.outcome IN ('unreadable', 'too_long', 'deadline', 'failed')
  AND t.attempt >= 3
  AND t.attempt = (SELECT MAX(later.attempt) FROM voice_transcript later
                   WHERE later.subject_id = t.subject_id AND later.note_id = t.note_id AND later.model = t.model)
  AND NOT EXISTS (SELECT 1 FROM voice_transcript_verdict judged
                  WHERE judged.subject_id = t.subject_id AND judged.note_id = t.note_id
                    AND judged.model = t.model AND judged.attempt = t.attempt AND judged.rule = 'heard-1');
