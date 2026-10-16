CREATE UNIQUE INDEX idx_assistant_actions_unique_pending
    ON assistant_actions(conversation_id, application_id, tool)
    WHERE status = 'pending';
