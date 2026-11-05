//! 共享 API 连接：保留两种用途的调度配置，但只引用主配置的凭据。
#![allow(clippy::unwrap_used)]

use prism_store::{NewApiProfile, Store};

fn connection() -> NewApiProfile {
    NewApiProfile {
        name: "共享连接".into(),
        purpose: "text".into(),
        access_type: "official".into(),
        provider_type: "openai_chat".into(),
        base_url: "http://127.0.0.1/v1".into(),
        model: "text-from-api".into(),
        model_is_custom: false,
        max_concurrency: 2,
        rpm_limit: None,
        weight: 1,
        timeout_secs: 120,
        transparent_method: "api".into(),
        capabilities: None,
        custom_spec: None,
    }
}

#[tokio::test]
async fn shared_profiles_update_atomically_and_reference_one_credential() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("test.db")).await.unwrap();
    let parent = store
        .save_profile_connection("connection", &connection(), Some("image-from-api"), 1)
        .await
        .unwrap();
    let child = store.list_profiles(Some("image")).await.unwrap().remove(0);
    assert_eq!(parent.credential_id(), child.credential_id());
    assert_eq!(child.provider_type, "openai_images");
    assert_eq!(child.model, "image-from-api");
    store
        .save_models_cache(&parent.id, "[\"text-from-api\",\"image-from-api\"]", 2)
        .await
        .unwrap();
    assert_eq!(
        store.get_profile(&parent.id).await.unwrap().models_cache,
        store.get_profile(&child.id).await.unwrap().models_cache
    );
    let mut updated = connection();
    updated.base_url = "http://127.0.0.1:18080/v1".into();
    store
        .save_profile_connection(&parent.id, &updated, Some("image-updated"), 3)
        .await
        .unwrap();
    let saved_child = store.get_profile(&child.id).await.unwrap();
    assert_eq!(saved_child.model, "image-updated");
    assert_eq!(saved_child.base_url, updated.base_url);
    assert_eq!(saved_child.models_cache, None);
    updated.purpose = "image".into();
    assert!(
        store
            .save_profile_connection(&parent.id, &updated, Some("bad"), 4)
            .await
            .is_err()
    );
    assert_eq!(store.get_profile(&child.id).await.unwrap(), saved_child);
    store.delete_profile(&parent.id).await.unwrap();
    assert!(store.list_profiles(None).await.unwrap().is_empty());
    store.close().await;
}

#[tokio::test]
async fn existing_single_profile_can_enable_and_disable_sharing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("test.db")).await.unwrap();
    let parent = store.create_profile(&connection(), 1).await.unwrap();
    store
        .save_profile_connection(&parent.id, &connection(), Some("image"), 2)
        .await
        .unwrap();
    assert_eq!(store.list_profiles(None).await.unwrap().len(), 2);
    store
        .save_profile_connection(&parent.id, &connection(), None, 3)
        .await
        .unwrap();
    assert_eq!(store.list_profiles(None).await.unwrap().len(), 1);
    assert_eq!(
        store.get_profile(&parent.id).await.unwrap().credential_id(),
        parent.id
    );
    store.close().await;
}
