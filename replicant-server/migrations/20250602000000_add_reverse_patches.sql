-- Add support for bidirectional patches in change_events
-- Migration: 20250602000000_add_reverse_patches

-- Rename existing json_patch column to be more explicit
ALTER TABLE change_events 
RENAME COLUMN json_patch TO forward_patch;

-- Add reverse_patch column for undo operations
ALTER TABLE change_events 
ADD COLUMN reverse_patch JSONB;

-- Add comment to clarify the purpose of each column
COMMENT ON COLUMN change_events.forward_patch IS 'JSON patch to apply this change (forward direction)';
COMMENT ON COLUMN change_events.reverse_patch IS 'JSON patch to undo this change (reverse direction)';

-- For CREATE events: forward_patch contains the initial document content, reverse_patch is null
-- For UPDATE events: forward_patch contains changes to apply, reverse_patch contains changes to undo
-- For DELETE events: forward_patch is null, reverse_patch contains the full document to restore