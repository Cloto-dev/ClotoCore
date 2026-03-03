-- Update Cloto Assistant description to reflect its full capabilities.
UPDATE agents SET description = 'Cloto''s default AI assistant. Operates as the primary interface between the user and the Cloto platform. Has access to persistent memory, tool execution, and web search capabilities. Communicates naturally and assists with system management, information retrieval, and general tasks.'
WHERE id = 'agent.cloto_default';
