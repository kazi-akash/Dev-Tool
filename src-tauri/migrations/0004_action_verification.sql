-- Action lifecycle results. Rows written before this migration have no verification data and stay UNKNOWN.
ALTER TABLE action_log ADD COLUMN reversibility TEXT NOT NULL DEFAULT 'UNKNOWN';
ALTER TABLE action_log ADD COLUMN status TEXT NOT NULL DEFAULT 'UNKNOWN';
ALTER TABLE action_log ADD COLUMN ram_recovered_bytes INTEGER;
ALTER TABLE action_log ADD COLUMN disk_recovered_bytes INTEGER;
