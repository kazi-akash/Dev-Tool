-- Developer build/startup sessions. Correlation with incidents is computed at query time.
CREATE TABLE build_sessions (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    start_ms      INTEGER NOT NULL,
    end_ms        INTEGER NOT NULL,
    kind          TEXT    NOT NULL,
    summary_json  TEXT    NOT NULL
);
CREATE INDEX idx_build_sessions_start ON build_sessions (start_ms DESC);
