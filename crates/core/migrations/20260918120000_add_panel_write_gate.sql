-- What a marketplace install recorded about the tree it placed, kept so the
-- panel write gate can ask later. A connector that ships no server (a
-- ui_module) gets no mcp_servers row, so before this table the trust level and
-- the seal of such a tree were computed at install and then dropped. Keyed by
-- the directory under the servers root, because that is the name a panel is
-- discovered under.
CREATE TABLE IF NOT EXISTS connector_install_receipts (
    connector_dir TEXT PRIMARY KEY,
    -- the catalog's trust level at install time
    trust_level   TEXT NOT NULL,
    -- the local tree seal minted at install; NULL when the tree was unsealed
    seal          TEXT,
    version       TEXT NOT NULL DEFAULT '',
    installed_at  TEXT NOT NULL
);

-- An operator's consent to one panel's declared writes. A consent holds only
-- while the declared list and the connector version are the ones consented to;
-- the gate compares both on every write, so an update that changes either
-- leaves the row in place but no longer valid.
CREATE TABLE IF NOT EXISTS panel_write_consents (
    panel_id          TEXT PRIMARY KEY,
    -- SHA-256 of the canonicalised `writes` list
    writes_digest     TEXT NOT NULL,
    connector_version TEXT NOT NULL,
    granted_at        TEXT NOT NULL,
    granted_by        TEXT NOT NULL
);
