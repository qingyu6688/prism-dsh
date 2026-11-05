-- 0003：保存的表格列映射，下次导入表头差不多的表格时直接套用。对应 docs/database.md 第 7 章。

CREATE TABLE import_mapping (
    id           TEXT    PRIMARY KEY,
    target       TEXT    NOT NULL,
    name         TEXT    NOT NULL,
    headers      TEXT    NOT NULL,
    columns      TEXT    NOT NULL,
    header_line  INTEGER,
    use_count    INTEGER NOT NULL DEFAULT 0,
    last_used_at INTEGER,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
CREATE INDEX idx_import_mapping_target ON import_mapping (target, last_used_at);
