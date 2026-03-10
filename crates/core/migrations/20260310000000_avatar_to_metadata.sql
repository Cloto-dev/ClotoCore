-- Phase 2: Data Sovereignty — migrate avatar/VRM columns to metadata JSON.
-- Eliminates domain-specific schema columns from the kernel (P4 compliance).

-- 1. Copy existing avatar/VRM data into the metadata JSON column
UPDATE agents SET metadata = json_set(
    COALESCE(metadata, '{}'),
    '$.avatar_path', avatar_path,
    '$.avatar_description', avatar_description,
    '$.vrm_path', vrm_path
)
WHERE avatar_path IS NOT NULL
   OR avatar_description IS NOT NULL
   OR vrm_path IS NOT NULL;

-- 2. Drop the dedicated columns (SQLite 3.35.0+)
ALTER TABLE agents DROP COLUMN avatar_path;
ALTER TABLE agents DROP COLUMN avatar_description;
ALTER TABLE agents DROP COLUMN vrm_path;
