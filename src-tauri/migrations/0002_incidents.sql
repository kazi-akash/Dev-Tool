-- Performance incidents: one summary row per recorded incident. Times are Unix milliseconds.
CREATE TABLE incidents (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    start_ms        INTEGER NOT NULL,
    end_ms          INTEGER NOT NULL,
    duration_secs   INTEGER NOT NULL,
    peak_ram_percent REAL   NOT NULL,
    mechanism       TEXT    NOT NULL,
    confidence      REAL    NOT NULL,
    diagnosis       TEXT    NOT NULL,
    summary_json    TEXT    NOT NULL
);
CREATE INDEX idx_incidents_start ON incidents (start_ms DESC);
