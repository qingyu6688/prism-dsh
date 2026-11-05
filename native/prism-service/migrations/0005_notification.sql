-- 0005：通知中心。计划等待确认、计划结束、一轮出错时记一条。对应 docs/database.md 第 9 章。

CREATE TABLE notification (
    id              TEXT    PRIMARY KEY,
    conversation_id TEXT    NOT NULL REFERENCES conversation (id) ON DELETE CASCADE,
    plan_id         TEXT    REFERENCES agent_plan (id) ON DELETE CASCADE,
    kind            TEXT    NOT NULL CHECK (kind IN ('plan_confirm', 'plan_finished', 'turn_failed')),
    text            TEXT    NOT NULL,
    read_at         INTEGER,
    created_at      INTEGER NOT NULL,
    UNIQUE (plan_id, kind)
);
CREATE INDEX idx_notification_created ON notification (created_at);
CREATE INDEX idx_notification_unread ON notification (read_at);
