-- Rename "Python Analyst" agent to "Python Bridge" for consistency with architecture docs
UPDATE agents
SET name = 'Python Bridge',
    description = 'Universal Python Bridge agent with async event streaming and data analysis capabilities'
WHERE id = 'agent.analyst';
