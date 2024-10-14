-- Add up migration script here
CREATE TABLE IF NOT EXISTS browser_tabs (
    id INTEGER PRIMARY KEY,
    status TEXT,
    tab_index INTEGER NOT NULL,
    opener_tab_id INTEGER,
    title TEXT,
    url TEXT,
    pending_url TEXT,
    pinned BOOLEAN NOT NULL,
    highlighted BOOLEAN NOT NULL,
    window_id INTEGER NOT NULL,
    active BOOLEAN NOT NULL,
    favicon_url TEXT,
    incognito BOOLEAN NOT NULL,
    selected BOOLEAN NOT NULL,
    audible BOOLEAN NOT NULL,
    discarded BOOLEAN NOT NULL,
    auto_discardable BOOLEAN NOT NULL,
    width INTEGER,
    height INTEGER,
    session_id TEXT,
    group_id INTEGER NOT NULL,
    last_accessed INTEGER NOT NULL,
    muted BOOLEAN NOT NULL,
    muted_reason TEXT,
    muted_extension_id TEXT
);

