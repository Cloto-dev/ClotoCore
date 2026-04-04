-- Add chain_hash column for Merkle chain integrity verification.
-- Each entry's chain_hash = SHA-256(previous_chain_hash | canonical_entry_data).
-- NULL for pre-existing entries (chain starts from first entry with non-NULL hash).
ALTER TABLE audit_logs ADD COLUMN chain_hash TEXT;
