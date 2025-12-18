-- Add title column as a derived field from content['title']
-- This is for database browsing and query performance only
-- The title is NOT synced independently and inherits conflict resolution from content

-- Add the column
ALTER TABLE documents ADD COLUMN title TEXT;

-- Backfill existing documents
-- If content has 'title', use it (truncated to 128 chars)
-- Otherwise, use formatted datetime: YYYY-MM-DD|HH:MM:SS.mmm
UPDATE documents SET title = COALESCE(
    substring(content->>'title', 1, 128),
    to_char(created_at, 'YYYY-MM-DD|HH24:MI:SS.MS')
);

-- Create index for query performance
CREATE INDEX idx_documents_title ON documents(title);
