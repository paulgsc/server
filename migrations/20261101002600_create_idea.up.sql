-- An idea is one record on home (STUDIO ST26: paulgsc/server#645, the phone's
-- half paulgsc/some-ui#2453): the title the owner types, the topic (the
-- owner's word is "subject"; the code avoids `SubjectId`'s name), the format
-- (`short` or `video`), its keywords, the version of the question set it
-- answers, and its answers, sent together by Send. Until now home held only
-- loose `studio` voice notes whose `ref` named an idea and a question, and
-- nothing that said what the idea was.
--
-- Every table here is a record, not a convenience copy, so every table is
-- APPEND-ONLY, as the owner's other records are (docs/identity.md invariants
-- 15-31). No row is ever updated (each trigger below aborts an UPDATE), and
-- the only deletes are account deletion's (invariant 12).
--
-- **One revision per send.** A send that changes the bundle adds a revision
-- with its answers; earlier revisions stay. The idea as home serves it is its
-- latest revision. A send held already (the same `sent_at` and the same
-- bundle), or one whose bundle is the latest revision's, writes nothing.
--
-- **An answer names a voice note by its id, or is typed.** The note must be
-- one of the subject's own `studio` notes home already holds, and not one the
-- owner set aside: its audio went home through `PUT /voice-notes/:id`
-- (invariant 23), and only there. Typed
-- text is a fallback for when voice failed, at most 1,000 characters.
--
-- **Marks sit beside the idea, never on it.** "Still furnishing" is a mark
-- the owner turns on and off: every change is a row, and the latest stands.
-- It is the owner's alone: nothing here reads it for a routine or the
-- mailbox. Setting an idea aside takes it out of the list for good: a mark,
-- once, never a delete.
--
-- **The lists the owner edits.** Topics and keywords start as a few entries
-- `idea_repo` names (`SEED_TOPICS`, `SEED_KEYWORDS`), and the owner adds and
-- takes off entries, each as a mark. Taking one off leaves the ideas that
-- use it as they are: a send is held to the lists only for what it adds.
--
-- Columns of `idea_revision`, one row per send that changed the idea:
--
--   subject_id -- whose record.
--   idea_id    -- the phone's id for the idea.
--   revision   -- 1 for the first send, then the next number per changed send.
--   created_at -- when the phone began the idea, ISO-8601 UTC to the
--                 millisecond; every revision of an idea carries its first's.
--   sent_at    -- when the owner pressed Send, ISO-8601 UTC to the millisecond;
--                 what tells a retried send from a new one.
--   title      -- the title the owner typed, trimmed: 1 to 80 characters.
--   topic      -- an entry of the topics list as it stood at the send, or
--                 the topic the idea's previous revision carried.
--   format     -- `short` (vertical) or `video` (wide).
--   keywords   -- a JSON array of entries of the keywords list as it stood at
--                 the send (or that the previous revision carried), each
--                 once, sorted.
--   questions  -- the version of the question set the answers answer.
--   stored_at  -- ISO-8601 UTC, when the revision reached home.
CREATE TABLE idea_revision (
    subject_id TEXT    NOT NULL,
    idea_id    TEXT    NOT NULL,
    revision   INTEGER NOT NULL CHECK (revision > 0),
    created_at TEXT    NOT NULL,
    sent_at    TEXT    NOT NULL,
    title      TEXT    NOT NULL,
    topic      TEXT    NOT NULL,
    format     TEXT    NOT NULL CHECK (format IN ('short', 'video')),
    keywords   TEXT    NOT NULL,
    questions  INTEGER NOT NULL,
    stored_at  TEXT    NOT NULL,
    PRIMARY KEY (subject_id, idea_id, revision)
);

-- The list: a subject's ideas, newest begun first. The primary key serves a
-- send's and a read's lookup by id.
CREATE INDEX idea_revision_created ON idea_revision (subject_id, created_at);

CREATE TRIGGER idea_revision_is_append_only
BEFORE UPDATE ON idea_revision
BEGIN
    SELECT RAISE(ABORT, 'idea_revision is append-only: send a new revision');
END;

CREATE TRIGGER idea_revision_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM idea_revision WHERE subject_id = OLD.subject_id;
END;

-- Columns of `idea_answer`, one row per question a revision answers:
--
--   subject_id -- whose record.
--   idea_id    -- the idea.
--   revision   -- the revision it belongs to.
--   question   -- the question's id in the revision's question set.
--   note_id    -- the `studio` voice note that answers it, by the phone's id
--                 (`voice_note.note_id`); none when typed.
--   typed      -- the answer typed when voice failed, at most 1,000
--                 characters; none when a note answers it.
CREATE TABLE idea_answer (
    subject_id TEXT    NOT NULL,
    idea_id    TEXT    NOT NULL,
    revision   INTEGER NOT NULL,
    question   TEXT    NOT NULL,
    note_id    TEXT,
    typed      TEXT,
    CHECK ((note_id IS NULL) <> (typed IS NULL)),
    PRIMARY KEY (subject_id, idea_id, revision, question)
);

CREATE TRIGGER idea_answer_is_append_only
BEFORE UPDATE ON idea_answer
BEGIN
    SELECT RAISE(ABORT, 'idea_answer is append-only: send a new revision');
END;

CREATE TRIGGER idea_answer_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM idea_answer WHERE subject_id = OLD.subject_id;
END;

-- Columns of `idea_furnishing`, one row per change of the mark:
--
--   subject_id -- whose record.
--   idea_id    -- the idea.
--   seq        -- 1 for the idea's first mark, then the next: the latest stands.
--   furnishing -- 1 while the owner is still furnishing it, 0 once not.
--   stored_at  -- ISO-8601 UTC, when home recorded the change.
CREATE TABLE idea_furnishing (
    subject_id TEXT    NOT NULL,
    idea_id    TEXT    NOT NULL,
    seq        INTEGER NOT NULL CHECK (seq > 0),
    furnishing INTEGER NOT NULL CHECK (furnishing IN (0, 1)),
    stored_at  TEXT    NOT NULL,
    PRIMARY KEY (subject_id, idea_id, seq)
);

CREATE TRIGGER idea_furnishing_is_append_only
BEFORE UPDATE ON idea_furnishing
BEGIN
    SELECT RAISE(ABORT, 'idea_furnishing is append-only: mark it again');
END;

CREATE TRIGGER idea_furnishing_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM idea_furnishing WHERE subject_id = OLD.subject_id;
END;

-- Columns of `idea_set_aside`, one row per idea the owner set aside:
--
--   subject_id -- whose record.
--   idea_id    -- the idea.
--   stored_at  -- ISO-8601 UTC, when home recorded the mark. The first mark
--                 stands; there is no un-set.
CREATE TABLE idea_set_aside (
    subject_id TEXT NOT NULL,
    idea_id    TEXT NOT NULL,
    stored_at  TEXT NOT NULL,
    PRIMARY KEY (subject_id, idea_id)
);

CREATE TRIGGER idea_set_aside_is_append_only
BEFORE UPDATE ON idea_set_aside
BEGIN
    SELECT RAISE(ABORT, 'idea_set_aside is append-only: the first mark stands');
END;

CREATE TRIGGER idea_set_aside_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM idea_set_aside WHERE subject_id = OLD.subject_id;
END;

-- Columns of `idea_list_mark`, one row per entry added to or taken off a list:
--
--   subject_id -- whose lists.
--   list       -- `topic` or `keyword`.
--   entry      -- the entry, as the owner wrote it: 1 to 40 characters.
--   seq        -- 1 for the entry's first mark, then the next: the latest
--                 stands.
--   listed     -- 1 when it was added, 0 when it was taken off.
--   stored_at  -- ISO-8601 UTC, when home recorded the mark.
CREATE TABLE idea_list_mark (
    subject_id TEXT    NOT NULL,
    list       TEXT    NOT NULL CHECK (list IN ('topic', 'keyword')),
    entry      TEXT    NOT NULL,
    seq        INTEGER NOT NULL CHECK (seq > 0),
    listed     INTEGER NOT NULL CHECK (listed IN (0, 1)),
    stored_at  TEXT    NOT NULL,
    PRIMARY KEY (subject_id, list, entry, seq)
);

CREATE TRIGGER idea_list_mark_is_append_only
BEFORE UPDATE ON idea_list_mark
BEGIN
    SELECT RAISE(ABORT, 'idea_list_mark is append-only: mark the entry again');
END;

CREATE TRIGGER idea_list_mark_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM idea_list_mark WHERE subject_id = OLD.subject_id;
END;
