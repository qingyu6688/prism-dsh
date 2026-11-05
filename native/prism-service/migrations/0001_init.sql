-- 棱镜初始表结构，对应 docs/database.md。
-- 已发布的迁移文件不再修改，字段变更新增 0002_xxx.sql。
-- 主键为 UUID v7 字符串；时间为 Unix 毫秒；JSON 字段存字符串。

PRAGMA foreign_keys = ON;

CREATE TABLE api_profile (
    id                 TEXT    PRIMARY KEY,
    name               TEXT    NOT NULL,
    purpose            TEXT    NOT NULL CHECK (purpose IN ('image', 'text')),
    access_type        TEXT    NOT NULL DEFAULT 'relay' CHECK (access_type IN ('official', 'relay')),
    provider_type      TEXT    NOT NULL CHECK (provider_type IN ('openai_images', 'openai_responses', 'openai_chat', 'async_task', 'custom_json')),
    base_url           TEXT    NOT NULL,
    model              TEXT    NOT NULL,
    model_is_custom    INTEGER NOT NULL DEFAULT 0,
    models_cache       TEXT,
    models_fetched_at  INTEGER,
    max_concurrency    INTEGER NOT NULL DEFAULT 5,
    rpm_limit          INTEGER,
    weight             INTEGER NOT NULL DEFAULT 1,
    timeout_secs       INTEGER NOT NULL DEFAULT 600,
    transparent_method TEXT    NOT NULL DEFAULT 'api' CHECK (transparent_method IN ('api', 'local')),
    capabilities       TEXT,
    custom_spec        TEXT,
    enabled            INTEGER NOT NULL DEFAULT 1,
    disabled_reason    TEXT,
    sort_order         INTEGER NOT NULL DEFAULT 0,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);
CREATE INDEX idx_api_profile_purpose_enabled ON api_profile (purpose, enabled);

CREATE TABLE conversation (
    id             TEXT    PRIMARY KEY,
    title          TEXT    NOT NULL,
    next_source_no INTEGER NOT NULL DEFAULT 1,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);
CREATE INDEX idx_conversation_updated_at ON conversation (updated_at);

CREATE TABLE message (
    id              TEXT    PRIMARY KEY,
    conversation_id TEXT    NOT NULL REFERENCES conversation (id) ON DELETE CASCADE,
    role            TEXT    NOT NULL CHECK (role IN ('user', 'assistant', 'tool', 'event')),
    content         TEXT    NOT NULL,
    status          TEXT    NOT NULL DEFAULT 'done' CHECK (status IN ('done', 'pending_confirm', 'confirmed', 'rejected')),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX idx_message_conversation ON message (conversation_id, created_at);

CREATE TABLE batch (
    id                  TEXT    PRIMARY KEY,
    seq                 INTEGER NOT NULL UNIQUE,
    conversation_id     TEXT    REFERENCES conversation (id) ON DELETE SET NULL,
    mode                TEXT    NOT NULL CHECK (mode IN ('similar', 'theme', 'palette', 'text')),
    variants_per_source INTEGER NOT NULL,
    params              TEXT    NOT NULL,
    status              TEXT    NOT NULL CHECK (status IN ('running', 'paused', 'completed', 'failed', 'cancelled')),
    pause_reason        TEXT,
    total_jobs          INTEGER NOT NULL DEFAULT 0,
    succeeded_jobs      INTEGER NOT NULL DEFAULT 0,
    failed_jobs         INTEGER NOT NULL DEFAULT 0,
    resubmitted_jobs    INTEGER NOT NULL DEFAULT 0,
    finished_at         INTEGER,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
);
CREATE INDEX idx_batch_status ON batch (status);
CREATE INDEX idx_batch_conversation ON batch (conversation_id);
CREATE INDEX idx_batch_created_at ON batch (created_at);

-- 再裂变时，新原图的 parent_image_id 指向某张结果图。image 表在后面创建，SQLite 允许这样的前向引用。
CREATE TABLE source_image (
    id              TEXT    PRIMARY KEY,
    batch_id        TEXT    NOT NULL REFERENCES batch (id) ON DELETE CASCADE,
    seq             INTEGER NOT NULL,
    ref_no          TEXT,
    parent_image_id TEXT    REFERENCES image (id) ON DELETE SET NULL,
    file_path       TEXT    NOT NULL,
    sha256          TEXT    NOT NULL,
    width           INTEGER NOT NULL,
    height          INTEGER NOT NULL,
    phash           TEXT,
    analysis        TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX idx_source_batch ON source_image (batch_id, seq);
CREATE INDEX idx_source_sha256 ON source_image (sha256);
CREATE INDEX idx_source_parent ON source_image (parent_image_id);

CREATE TABLE job (
    id             TEXT    PRIMARY KEY,
    batch_id       TEXT    NOT NULL REFERENCES batch (id) ON DELETE CASCADE,
    batch_seq      INTEGER NOT NULL,
    source_id      TEXT    NOT NULL REFERENCES source_image (id) ON DELETE CASCADE,
    source_seq     INTEGER NOT NULL,
    kind           TEXT    NOT NULL CHECK (kind IN ('plan', 'generate')),
    variant_index  INTEGER NOT NULL DEFAULT 0,
    axis           TEXT    CHECK (axis IN ('element', 'composition', 'palette', 'theme', 'text')),
    instruction    TEXT,
    rewritten      INTEGER NOT NULL DEFAULT 0,
    priority       INTEGER NOT NULL DEFAULT 0,
    status         TEXT    NOT NULL CHECK (status IN ('queued', 'running', 'polling', 'retry_wait', 'succeeded', 'failed', 'cancelled')),
    attempt        INTEGER NOT NULL DEFAULT 0,
    max_attempts   INTEGER NOT NULL DEFAULT 3,
    next_run_at    INTEGER,
    profile_id     TEXT    REFERENCES api_profile (id) ON DELETE SET NULL,
    remote_task_id TEXT,
    request_params TEXT,
    error_code     TEXT,
    error_message  TEXT,
    started_at     INTEGER,
    finished_at    INTEGER,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);
CREATE INDEX idx_job_dispatch ON job (status, priority, kind, variant_index, batch_seq, source_seq);
CREATE INDEX idx_job_retry ON job (status, next_run_at);
CREATE INDEX idx_job_batch ON job (batch_id, status);
CREATE INDEX idx_job_source ON job (source_id, variant_index);

CREATE TABLE image (
    id             TEXT    PRIMARY KEY,
    job_id         TEXT    NOT NULL REFERENCES job (id) ON DELETE CASCADE,
    batch_id       TEXT    NOT NULL REFERENCES batch (id) ON DELETE CASCADE,
    source_id      TEXT    NOT NULL REFERENCES source_image (id) ON DELETE CASCADE,
    variant_index  INTEGER NOT NULL,
    ref_no         TEXT,
    file_path      TEXT    NOT NULL,
    thumb_path     TEXT,
    format         TEXT    NOT NULL CHECK (format IN ('png', 'webp', 'jpg')),
    width          INTEGER NOT NULL,
    height         INTEGER NOT NULL,
    sha256         TEXT    NOT NULL,
    phash          TEXT,
    dist_to_source INTEGER,
    flags          TEXT,
    actual_params  TEXT,
    starred        INTEGER NOT NULL DEFAULT 0,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);
CREATE INDEX idx_image_source ON image (source_id, variant_index);
CREATE INDEX idx_image_batch ON image (batch_id);
CREATE INDEX idx_image_starred ON image (starred);

CREATE TABLE setting (
    id         TEXT    PRIMARY KEY,
    value      TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
