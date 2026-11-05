-- no-transaction
-- 第一行的 no-transaction 告诉迁移程序：在事务外执行本文件，并在执行前关闭外键检查。
--
-- 0002：通用平台的表（商品、规格、店铺、素材、标签、文案），批次和任务改成各模块通用。
-- 对应 docs/database.md 第 6 章。
--
-- batch 表要去掉 mode 的 CHECK、job 表要把 source_id 改成可空，SQLite 只能重建表，按官方步骤
-- （https://www.sqlite.org/lang_altertable.html#otheralter）：外键关闭 → 事务里建新表、复制、删旧表、
-- 改名、重建索引 → 提交 → 检查外键 → 打开外键。外键开着时删旧表会级联删除所有结果图，
-- 改表名也会把 image 表的外键改写到旧表上，所以必须关外键（迁移程序负责关和开）。

BEGIN;

CREATE TABLE shop (
    id         TEXT    PRIMARY KEY,
    name       TEXT    NOT NULL,
    platform   TEXT    NOT NULL,
    site       TEXT,
    language   TEXT    NOT NULL DEFAULT 'zh-CN',
    currency   TEXT    NOT NULL DEFAULT 'CNY',
    notes      TEXT,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX idx_shop_platform ON shop (platform);

CREATE TABLE product (
    id             TEXT    PRIMARY KEY,
    code           TEXT,
    name           TEXT    NOT NULL,
    category       TEXT,
    brand          TEXT,
    description    TEXT,
    selling_points TEXT,
    attributes     TEXT,
    cost_cents     INTEGER,
    currency       TEXT    NOT NULL DEFAULT 'CNY',
    target_markets TEXT,
    status         TEXT    NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived')),
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);
CREATE UNIQUE INDEX idx_product_code ON product (code) WHERE code IS NOT NULL;
CREATE INDEX idx_product_status_updated ON product (status, updated_at);
CREATE INDEX idx_product_name ON product (name);

CREATE TABLE product_sku (
    id          TEXT    PRIMARY KEY,
    product_id  TEXT    NOT NULL REFERENCES product (id) ON DELETE CASCADE,
    code        TEXT,
    spec        TEXT    NOT NULL,
    price_cents INTEGER,
    cost_cents  INTEGER,
    stock       INTEGER,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX idx_sku_product ON product_sku (product_id, sort_order);
CREATE INDEX idx_sku_code ON product_sku (code);

-- 批次：重建表，新增所属模块、关联商品；mode 去掉只允许图裂变四种模式的 CHECK，
-- 各模块的子模式由程序校验
CREATE TABLE batch_new (
    id                  TEXT    PRIMARY KEY,
    seq                 INTEGER NOT NULL UNIQUE,
    conversation_id     TEXT    REFERENCES conversation (id) ON DELETE SET NULL,
    module              TEXT    NOT NULL DEFAULT 'image' CHECK (module IN ('image', 'listing', 'analytics', 'research')),
    product_id          TEXT    REFERENCES product (id) ON DELETE SET NULL,
    mode                TEXT    NOT NULL,
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

INSERT INTO batch_new (
    id, seq, conversation_id, module, product_id, mode, variants_per_source, params, status,
    pause_reason, total_jobs, succeeded_jobs, failed_jobs, resubmitted_jobs, finished_at,
    created_at, updated_at
)
SELECT
    id, seq, conversation_id, 'image', NULL, mode, variants_per_source, params, status,
    pause_reason, total_jobs, succeeded_jobs, failed_jobs, resubmitted_jobs, finished_at,
    created_at, updated_at
FROM batch;

DROP TABLE batch;
ALTER TABLE batch_new RENAME TO batch;

CREATE INDEX idx_batch_status ON batch (status);
CREATE INDEX idx_batch_conversation ON batch (conversation_id);
CREATE INDEX idx_batch_created_at ON batch (created_at);
CREATE INDEX idx_batch_module_created ON batch (module, created_at);

-- 任务：重建表，source_id 改为可空，新增 task_type、product_id
CREATE TABLE job_new (
    id             TEXT    PRIMARY KEY,
    batch_id       TEXT    NOT NULL REFERENCES batch (id) ON DELETE CASCADE,
    batch_seq      INTEGER NOT NULL,
    source_id      TEXT    REFERENCES source_image (id) ON DELETE CASCADE,
    source_seq     INTEGER NOT NULL DEFAULT 0,
    kind           TEXT    NOT NULL CHECK (kind IN ('plan', 'generate')),
    task_type      TEXT    NOT NULL DEFAULT 'image.fission',
    product_id     TEXT    REFERENCES product (id) ON DELETE SET NULL,
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

INSERT INTO job_new (
    id, batch_id, batch_seq, source_id, source_seq, kind, task_type, product_id, variant_index,
    axis, instruction, rewritten, priority, status, attempt, max_attempts, next_run_at, profile_id,
    remote_task_id, request_params, error_code, error_message, started_at, finished_at,
    created_at, updated_at
)
SELECT
    id, batch_id, batch_seq, source_id, source_seq, kind, 'image.fission', NULL, variant_index,
    axis, instruction, rewritten, priority, status, attempt, max_attempts, next_run_at, profile_id,
    remote_task_id, request_params, error_code, error_message, started_at, finished_at,
    created_at, updated_at
FROM job;

DROP TABLE job;
ALTER TABLE job_new RENAME TO job;

CREATE INDEX idx_job_dispatch ON job (status, priority, kind, variant_index, batch_seq, source_seq);
CREATE INDEX idx_job_retry ON job (status, next_run_at);
CREATE INDEX idx_job_batch ON job (batch_id, status);
CREATE INDEX idx_job_source ON job (source_id, variant_index);
CREATE INDEX idx_job_product ON job (product_id);

CREATE TABLE asset (
    id              TEXT    PRIMARY KEY,
    kind            TEXT    NOT NULL CHECK (kind IN ('image', 'text')),
    product_id      TEXT    REFERENCES product (id) ON DELETE SET NULL,
    source          TEXT    NOT NULL CHECK (source IN ('upload', 'generated', 'captured', 'imported')),
    title           TEXT,
    file_path       TEXT,
    thumb_path      TEXT,
    text_content    TEXT,
    mime            TEXT,
    width           INTEGER,
    height          INTEGER,
    sha256          TEXT,
    phash           TEXT,
    origin_batch_id TEXT    REFERENCES batch (id) ON DELETE SET NULL,
    origin_job_id   TEXT    REFERENCES job (id) ON DELETE SET NULL,
    meta            TEXT,
    starred         INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    CHECK (
        (kind = 'image' AND file_path IS NOT NULL) OR
        (kind = 'text' AND text_content IS NOT NULL)
    )
);
CREATE INDEX idx_asset_product ON asset (product_id, kind);
CREATE INDEX idx_asset_kind_created ON asset (kind, created_at);
CREATE INDEX idx_asset_sha256 ON asset (sha256);
CREATE INDEX idx_asset_starred ON asset (starred);

CREATE TABLE asset_tag (
    asset_id   TEXT    NOT NULL REFERENCES asset (id) ON DELETE CASCADE,
    tag        TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (asset_id, tag)
);
CREATE INDEX idx_asset_tag_tag ON asset_tag (tag);

CREATE TABLE product_copy (
    id            TEXT    PRIMARY KEY,
    product_id    TEXT    NOT NULL REFERENCES product (id) ON DELETE CASCADE,
    shop_id       TEXT    REFERENCES shop (id) ON DELETE SET NULL,
    language      TEXT    NOT NULL,
    field         TEXT    NOT NULL CHECK (field IN ('title', 'bullets', 'description', 'keywords', 'attributes', 'selling_points')),
    content       TEXT    NOT NULL,
    status        TEXT    NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'approved')),
    origin_job_id TEXT    REFERENCES job (id) ON DELETE SET NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);
CREATE INDEX idx_copy_lookup ON product_copy (product_id, shop_id, language, field);

COMMIT;
