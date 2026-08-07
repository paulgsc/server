-- A PushSubscription is the browser's return address. Losing one costs a
-- person a trip back to the site to re-subscribe, so it is stored rather
-- than held in memory.

CREATE TABLE push_subscriptions (
    -- The endpoint is unique per browser-per-origin and is the identity a
    -- 410 Gone later reports back, so it is the natural primary key.
    endpoint        TEXT    PRIMARY KEY,

    -- Whose it is. Auth has not landed; every row currently carries the same
    -- singleton subject. It is a column rather than a later migration because
    -- retrofitting an owner onto rows that never had one means guessing, and
    -- because everything downstream is keyed by subject already.
    subject_id      TEXT    NOT NULL,

    -- Both are base64url strings handed over verbatim by
    -- `registration.pushManager.subscribe()`.
    p256dh          TEXT    NOT NULL,
    auth            TEXT    NOT NULL,

    -- Explicit permission, recorded rather than assumed. The null case is no
    -- notifications at all: a row exists only because someone was asked and
    -- said yes, and this is the JSON array of what they said yes *to*. An
    -- empty array is a subscription that receives nothing, which is a
    -- meaningful state and not a bug.
    topics          TEXT    NOT NULL DEFAULT '[]',
    consented_at    TEXT    NOT NULL,

    created_at      TEXT    NOT NULL,   -- ISO-8601
    last_success_at TEXT,               -- ISO-8601; last 2xx from the push service
    last_failure_at TEXT,               -- ISO-8601; last non-2xx
    failure_count   INTEGER NOT NULL DEFAULT 0
);

-- The fan-out reads every subscription for one subject.
CREATE INDEX idx_push_subscriptions_subject ON push_subscriptions(subject_id);

-- Consecutive failures are the dimension a human looks at when asking why a
-- device stopped receiving anything.
CREATE INDEX idx_push_subscriptions_failure_count ON push_subscriptions(failure_count DESC);
