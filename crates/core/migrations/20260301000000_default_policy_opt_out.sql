-- Change default policy from opt-in (deny) to opt-out (allow).
-- All existing servers are updated to match the new default.
-- New servers will get 'opt-out' via save_mcp_server().
UPDATE mcp_servers SET default_policy = 'opt-out' WHERE default_policy = 'opt-in';
