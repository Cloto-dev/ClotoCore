-- Add MessageSource selection to CRON jobs.
-- source_type: 'system' (default, backward-compatible) or 'user'
-- When 'user', creator_user_id and creator_user_name are used to construct
-- MessageSource::User at dispatch time.
ALTER TABLE cron_jobs ADD COLUMN source_type TEXT NOT NULL DEFAULT 'system';
ALTER TABLE cron_jobs ADD COLUMN creator_user_id TEXT;
ALTER TABLE cron_jobs ADD COLUMN creator_user_name TEXT;
