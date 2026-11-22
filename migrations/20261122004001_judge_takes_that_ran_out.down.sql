-- Reverses 20261122004001_judge_takes_that_ran_out.up.sql by keeping what it
-- wrote: `voice_transcript_verdict` is append-only, and each row it added is
-- the verdict `judge` would have kept for that attempt, so it stays. Nothing
-- else was changed.
SELECT 1;
