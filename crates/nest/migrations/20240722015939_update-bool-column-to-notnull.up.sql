-- Add up migration script here
-- comment it out if no tansaction
-- ROLLBACK;

ALTER TABLE browser_tabs ADD COLUMN new_pinned BOOLEAN NOT NULL DEFAULT 0;
ALTER TABLE browser_tabs ADD COLUMN new_highlighted BOOLEAN NOT NULL DEFAULT 0;
ALTER TABLE browser_tabs ADD column new_active BOOLEAN NOT NULL DEFAULT 0;
ALTER TABLE browser_tabs ADD column new_incognito BOOLEAN NOT NULL DEFAULT 0;
ALTER TABLE browser_tabs ADD column new_selected BOOLEAN NOT NULL DEFAULT 0;
ALTER TABLE browser_tabs ADD column new_audible BOOLEAN NOT NULL DEFAULT 0;
ALTER TABLE browser_tabs ADD column new_discarded BOOLEAN NOT NULL DEFAULT 0;
ALTER TABLE browser_tabs ADD column new_auto_discardable BOOLEAN NOT NULL DEFAULT 0;  
UPDATE browser_tabs SET new_pinned = pinned;
UPDATE browser_tabs SET new_highlighted = highlighted;
UPDATE browser_tabs SET new_active = active;
UPDATE browser_tabs SET new_incognito = incognito;
UPDATE browser_tabs SET new_selected = selected;
UPDATE browser_tabs SET new_audible = audible;
UPDATE browser_tabs SET new_discarded = discarded;
UPDATE browser_tabs SET new_auto_discardable = auto_discardable;


ALTER TABLE browser_tabs DROP COLUMN pinned;
ALTER TABLE browser_tabs DROP COLUMN highlighted;
ALTER TABLE browser_tabs DROP COLUMN active;
ALTER TABLE browser_tabs DROP COLUMN incognito;
ALTER TABLE browser_tabs DROP COLUMN selected;
ALTER TABLE browser_tabs DROP COLUMN audible;
ALTER TABLE browser_tabs DROP COLUMN discarded;
ALTER TABLE browser_tabs DROP COLUMN auto_discardable;


ALTER TABLE browser_tabs RENAME COLUMN new_pinned TO pinned;
ALTER TABLE browser_tabs RENAME COLUMN new_highlighted TO highlighted;
ALTER TABLE browser_tabs RENAME COLUMN new_active TO active;
ALTER TABLE browser_tabs RENAME COLUMN new_incognito TO incognito;
ALTER TABLE browser_tabs RENAME COLUMN new_selected TO selected;
ALTER TABLE browser_tabs RENAME COLUMN new_audible TO audible;
ALTER TABLE browser_tabs RENAME COLUMN new_discarded TO discarded;
ALTER TABLE browser_tabs RENAME COLUMN new_auto_discardable TO auto_discardable;

