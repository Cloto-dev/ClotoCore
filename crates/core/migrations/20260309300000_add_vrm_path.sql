-- Add vrm_path column to agents table for VRM 3D avatar model storage
ALTER TABLE agents ADD COLUMN vrm_path TEXT DEFAULT NULL;
