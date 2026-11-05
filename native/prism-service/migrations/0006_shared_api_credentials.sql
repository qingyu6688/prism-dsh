-- 一份连接可同时提供文本和生图配置；生图子配置复用主配置在凭据管理器中的 Key。
ALTER TABLE api_profile ADD COLUMN credential_owner_id TEXT REFERENCES api_profile(id) ON DELETE CASCADE;
CREATE UNIQUE INDEX idx_api_profile_credential_owner ON api_profile(credential_owner_id) WHERE credential_owner_id IS NOT NULL;
