-- Add stdout, stderr, and exit_code columns to task_states table
-- These columns store the output from command execution

ALTER TABLE task_states ADD COLUMN stdout TEXT;
ALTER TABLE task_states ADD COLUMN stderr TEXT;
ALTER TABLE task_states ADD COLUMN exit_code INTEGER;
