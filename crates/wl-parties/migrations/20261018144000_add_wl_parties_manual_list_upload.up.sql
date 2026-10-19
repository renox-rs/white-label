-- Manual sanctions list uploads (wl-parties §5.4, issue #42): who uploaded
-- the current version of a list, from which page, and a log of every upload.
-- The user who uploaded the current version; none for a download.
ALTER TABLE "wl_parties_sanctions_lists" ADD COLUMN updated_by INTEGER;
-- The page of the authority the current version was taken from.
ALTER TABLE "wl_parties_sanctions_lists" ADD COLUMN source_url TEXT;
-- The file format of the list: `xml`, `pdf` or `excel`.
ALTER TABLE "wl_parties_sanctions_lists" ADD COLUMN format TEXT NOT NULL DEFAULT 'xml';

CREATE TABLE "wl_parties_sanctions_list_uploads" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    list_id INTEGER NOT NULL REFERENCES "wl_parties_sanctions_lists" (id) ON DELETE CASCADE,
    uploaded_by INTEGER NOT NULL,
    uploaded_at TEXT NOT NULL,
    -- As given; may be missing in a refused upload.
    source_url TEXT,
    -- ISO date `YYYY-MM-DD` as given; may be missing in a refused upload.
    list_date TEXT,
    file_name TEXT NOT NULL,
    -- Where the file is on the storage disk; none when it was not kept.
    file_ref TEXT,
    byte_size INTEGER NOT NULL,
    -- `accepted` or `refused`.
    outcome TEXT NOT NULL,
    refusal_reason TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_parties_sanctions_list_uploads_list" ON "wl_parties_sanctions_list_uploads" (list_id);
