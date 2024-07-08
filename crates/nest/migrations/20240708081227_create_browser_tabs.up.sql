-- Add up migration script here
CREATE TABLE browser_tabs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    status TEXT,
    "index" INTEGER,
    opener_tab_id INTEGER,
    title TEXT,
    url TEXT,
    pending_url TEXT,
    pinned BOOLEAN,
    highlighted BOOLEAN,
    window_id INTEGER,
    active BOOLEAN,
    fav_icon_url TEXT,
    incognito BOOLEAN,
    selected BOOLEAN,
    audible BOOLEAN,
    discarded BOOLEAN,
    auto_discardable BOOLEAN,
    muted_info TEXT,
    width INTEGER,
    height INTEGER,
    last_accessed TIMESTAMP
);