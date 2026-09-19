-- The archive digest the catalog advertised when the tree was installed.
-- The marketplace compares it with the catalog's current digest to offer an
-- update for a republish that kept its version. A connector that registers a
-- server already records this on its mcp_servers row; one that ships only
-- panels has no row, so without this column a same-version republish of it
-- was never offered. NULL when the catalog carried no digest, or for a
-- receipt written before this column existed.
ALTER TABLE connector_install_receipts ADD COLUMN archive_sha256 TEXT;
