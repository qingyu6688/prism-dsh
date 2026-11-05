-- 0004：Agent 一句话拆出的步骤计划和每一步的状态。对应 docs/database.md 第 8 章。

CREATE TABLE agent_plan (
    id             TEXT    PRIMARY KEY,
    conversation_id TEXT   NOT NULL REFERENCES conversation (id) ON DELETE CASCADE,
    message_id     TEXT    REFERENCES message (id) ON DELETE SET NULL,
    tool_call_id   TEXT    NOT NULL,
    status         TEXT    NOT NULL CHECK (status IN ('pending_confirm', 'running', 'succeeded', 'partial', 'failed', 'cancelled')),
    estimated_jobs INTEGER NOT NULL DEFAULT 0,
    finished_at    INTEGER,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);
CREATE INDEX idx_agent_plan_conversation ON agent_plan (conversation_id, created_at);
CREATE INDEX idx_agent_plan_status ON agent_plan (status);

CREATE TABLE agent_step (
    id             TEXT    PRIMARY KEY,
    plan_id        TEXT    NOT NULL REFERENCES agent_plan (id) ON DELETE CASCADE,
    step_key       TEXT    NOT NULL,
    seq            INTEGER NOT NULL,
    tool           TEXT    NOT NULL,
    title          TEXT    NOT NULL,
    arguments      TEXT    NOT NULL,
    depends_on     TEXT    NOT NULL DEFAULT '[]',
    status         TEXT    NOT NULL CHECK (status IN ('pending', 'running', 'succeeded', 'failed', 'skipped', 'cancelled')),
    estimated_jobs INTEGER NOT NULL DEFAULT 0,
    handle         TEXT,
    output         TEXT,
    error          TEXT,
    started_at     INTEGER,
    finished_at    INTEGER,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    UNIQUE (plan_id, step_key)
);
CREATE INDEX idx_agent_step_plan ON agent_step (plan_id, seq);
CREATE INDEX idx_agent_step_handle ON agent_step (handle);
