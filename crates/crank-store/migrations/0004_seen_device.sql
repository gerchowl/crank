-- #444 (ADR-0003 §2.3): the identities of devices a scan, a pairing attempt or a connect has seen,
-- kept only so the bug report can replace them in logs, rotated ones included, after the pairing is
-- forgotten. Absent values are '' so the key is plain; the writer keeps the newest 64 rows.
CREATE TABLE seen_device (
    kind TEXT NOT NULL,
    platform_id TEXT NOT NULL,
    name TEXT NOT NULL DEFAULT '',
    serial TEXT NOT NULL DEFAULT '',
    last_seen INTEGER NOT NULL,
    PRIMARY KEY (kind, platform_id, name, serial)
) WITHOUT ROWID;
