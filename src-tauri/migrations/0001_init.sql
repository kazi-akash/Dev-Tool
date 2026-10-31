-- DevPilot history. All times are Unix milliseconds.
-- Raw samples are short-lived; they are rolled up into 1 minute, 5 minute and 1 hour buckets.

CREATE TABLE metrics_raw (
    ts_ms        INTEGER PRIMARY KEY,
    ram_percent  REAL    NOT NULL,
    cpu_percent  REAL    NOT NULL,
    ram_used     INTEGER NOT NULL,
    ram_total    INTEGER NOT NULL,
    disks_json   TEXT    NOT NULL,
    top_json     TEXT    NOT NULL
);

CREATE TABLE metrics_agg (
    bucket_secs   INTEGER NOT NULL,
    ts_ms         INTEGER NOT NULL,
    samples       INTEGER NOT NULL,
    ram_avg       REAL    NOT NULL,
    ram_max       REAL    NOT NULL,
    cpu_avg       REAL    NOT NULL,
    cpu_max       REAL    NOT NULL,
    ram_used_avg  INTEGER NOT NULL,
    ram_total     INTEGER NOT NULL,
    disks_json    TEXT    NOT NULL,
    top_json      TEXT    NOT NULL,
    PRIMARY KEY (bucket_secs, ts_ms)
);

CREATE TABLE diagnoses (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    ts_ms          INTEGER NOT NULL,
    status         TEXT    NOT NULL,
    severity       TEXT    NOT NULL,
    rule_id        TEXT    NOT NULL,
    title          TEXT    NOT NULL,
    summary        TEXT    NOT NULL,
    primary_cause  TEXT,
    confidence     REAL    NOT NULL
);
CREATE INDEX idx_diagnoses_ts ON diagnoses (ts_ms DESC);

CREATE TABLE action_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    ts_ms        INTEGER NOT NULL,
    action_id    TEXT    NOT NULL,
    name         TEXT    NOT NULL,
    risk         TEXT    NOT NULL,
    automated    INTEGER NOT NULL,
    success      INTEGER NOT NULL,
    summary      TEXT    NOT NULL,
    freed_bytes  INTEGER
);
CREATE INDEX idx_action_log_ts ON action_log (ts_ms DESC);
