-- Grant MemoryRead and MemoryWrite to the memory.ks22 memory provider.
-- These permissions are now enforced by system.rs before recall/store operations.
-- Without them, memory recall and storage are blocked by the permission check.
UPDATE plugin_settings
SET allowed_permissions = '["MemoryRead","MemoryWrite"]'
WHERE plugin_id = 'memory.ks22'
  AND allowed_permissions = '[]';
