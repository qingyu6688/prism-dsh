//! 从接口拉模型列表，规则见 `docs/detail-design.md` 3.4。

use prism_core::defaults::DEFAULT_IMAGE_MODEL;
use serde_json::Value;

use crate::http::{authorize, build_client, send_json};
use crate::{ProviderConfig, ProviderError};

/// 模型列表。`supported = false` 表示接口不支持 `/models`，界面提示用户手动输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelList {
    pub supported: bool,
    /// 生图模型，默认模型排第一
    pub image_models: Vec<String>,
    pub other_models: Vec<String>,
}

impl ModelList {
    /// 新建生图配置时默认选哪个：列表里有默认模型就用它，否则用第一个生图模型。
    pub fn default_image_model(&self) -> Option<&str> {
        self.image_models.first().map(String::as_str)
    }
}

/// ID 里有 `image`，或以 `dall-e` 开头的算生图模型。
pub fn is_image_model(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    id.contains("image") || id.starts_with("dall-e")
}

/// 分组排序：默认模型排第一，其余按名字倒序（新版本通常排前面）。
pub fn sort_models(ids: impl IntoIterator<Item = String>) -> ModelList {
    let mut ids: Vec<String> = ids.into_iter().collect();
    ids.sort_by(|a, b| b.cmp(a));
    ids.dedup();
    let (mut image_models, other_models): (Vec<_>, Vec<_>) =
        ids.into_iter().partition(|id| is_image_model(id));
    if let Some(position) = image_models.iter().position(|id| id == DEFAULT_IMAGE_MODEL) {
        let default = image_models.remove(position);
        image_models.insert(0, default);
    }
    ModelList {
        supported: true,
        image_models,
        other_models,
    }
}

/// 调 `GET {base_url}/models`。接口返回 404 / 405 或格式不对时视为不支持，不算错误。
pub async fn list_models(config: &ProviderConfig) -> Result<ModelList, ProviderError> {
    let client = build_client(config)?;
    let request = authorize(client.get(config.endpoint("models")), config, None);
    let unsupported = ModelList {
        supported: false,
        image_models: Vec::new(),
        other_models: Vec::new(),
    };
    let body = match send_json(request, "models").await {
        Ok(body) => body,
        Err(error) if matches!(error.status, Some(404 | 405)) => return Ok(unsupported),
        Err(error) if error.code == prism_core::UpstreamErrorCode::BadResponse => {
            return Ok(unsupported);
        }
        Err(error) => return Err(error),
    };
    let Some(items) = body.get("data").and_then(Value::as_array) else {
        return Ok(unsupported);
    };
    let ids = items
        .iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .map(str::to_owned);
    Ok(sort_models(ids))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_image_models() {
        assert!(is_image_model("gpt-image-2.5-sunburst"));
        assert!(is_image_model("dall-e-3"));
        assert!(!is_image_model("gpt-4.1"));
    }

    #[test]
    fn default_model_goes_first() {
        let list = sort_models(
            [
                "gpt-image-1",
                "gpt-4.1",
                "gpt-image-2.5-sunburst",
                "gpt-image-2",
                "gpt-image-1",
            ]
            .map(String::from),
        );
        assert_eq!(
            list.image_models,
            vec!["gpt-image-2.5-sunburst", "gpt-image-2", "gpt-image-1"]
        );
        assert_eq!(list.other_models, vec!["gpt-4.1"]);
        assert_eq!(list.default_image_model(), Some("gpt-image-2.5-sunburst"));
    }

    #[test]
    fn falls_back_to_first_image_model_without_default() {
        let list = sort_models(["relay-image-v3", "relay-image-v2"].map(String::from));
        assert_eq!(list.default_image_model(), Some("relay-image-v3"));
    }
}
