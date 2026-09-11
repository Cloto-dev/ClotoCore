-- The store every waiting item lands in: approvals that hold an agent, proposals
-- an agent raises without holding anything, and notices that only inform.
--
-- It exists because the events that carry these already fly past on the bus and
-- are gone. An item a reader has not seen yet has to outlive the moment it was
-- produced, and outlive a restart, or "you were asked" is only true for whoever
-- happened to be looking at the screen.
--
-- severity holds the RFC 5424 identifier (debug/info/notice/warning/error/
-- critical/alert/emergency) — the same set the MCP logging levels already use.
-- Storing a separate three-value scale here would mean hand-maintaining a table
-- against those eight, so the display scale is derived at the edge instead.
--
-- blocking says whether this item is holding an agent. It is deliberately not a
-- function of severity: a reader who filters by severity must still be able to
-- see everything that is stuck, or the filter becomes a switch that starves
-- agents silently.
CREATE TABLE IF NOT EXISTS notifications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- Stable id chosen by the producer (an approval id, a call id). Lets the
    -- resolving side find the row it wrote without carrying a rowid around.
    item_id TEXT NOT NULL UNIQUE,
    -- 'approval' | 'proposal' | 'notice'. Named kind rather than type because
    -- the Rust side cannot spell `type`, and one word for both is worth more
    -- than matching the sketch.
    kind TEXT NOT NULL,
    severity TEXT NOT NULL,
    agent_id TEXT,
    title TEXT NOT NULL,
    body TEXT,
    created_at TEXT NOT NULL,
    read_at TEXT,
    resolved_at TEXT,
    -- What the item settled as, in the producer's own words ("approved",
    -- "denied by user", "channel closed"). Free text: the kernel already records
    -- these strings in its events and audit log, and a closed enum here would
    -- drift from them.
    decision TEXT,
    blocking INTEGER NOT NULL DEFAULT 0,
    metadata TEXT
);

-- The bell reads unresolved items; the badge reads unread ones.
CREATE INDEX IF NOT EXISTS idx_notifications_unresolved ON notifications(resolved_at, created_at);
CREATE INDEX IF NOT EXISTS idx_notifications_unread ON notifications(read_at, created_at);
CREATE INDEX IF NOT EXISTS idx_notifications_agent ON notifications(agent_id, created_at);
