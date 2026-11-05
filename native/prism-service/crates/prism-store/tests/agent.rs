//! 会话、消息和 Agent 步骤计划（迁移 0004）的存储测试。

// 测试里的辅助函数出错就该直接失败，用 unwrap 最直接
#![allow(clippy::unwrap_used, clippy::expect_used)]

use prism_store::{NewPlan, NewStep, Store, StoreError};
use tempfile::TempDir;

const T0: i64 = 1_800_000_000_000;

async fn open() -> (Store, TempDir) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("prism.db")).await.unwrap();
    (store, dir)
}

fn step(key: &str, depends_on: &str) -> NewStep {
    NewStep {
        step_key: key.into(),
        tool: "test.work".into(),
        title: format!("步骤 {key}"),
        arguments: "{}".into(),
        depends_on: depends_on.into(),
        estimated_jobs: 5,
    }
}

/// 等待确认的计划。
fn plan(conversation_id: &str, steps: Vec<NewStep>) -> NewPlan {
    NewPlan {
        conversation_id: conversation_id.into(),
        message_id: None,
        tool_call_id: "call".into(),
        status: "pending_confirm".into(),
        estimated_jobs: 0,
        steps,
    }
}

#[tokio::test]
async fn conversations_and_messages() {
    let (store, _dir) = open().await;
    let first = store.create_conversation("新会话", T0).await.unwrap();
    let second = store.create_conversation("另一个", T0 + 1).await.unwrap();

    // 同一毫秒追加的消息按插入顺序
    for text in ["一", "二", "三"] {
        store
            .append_message(
                &first.id,
                "user",
                &format!("{{\"text\":\"{text}\"}}"),
                "done",
                T0 + 5,
            )
            .await
            .unwrap();
    }
    let recent = store.recent_messages(&first.id, 2).await.unwrap();
    let texts: Vec<&str> = recent.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(texts, [r#"{"text":"二"}"#, r#"{"text":"三"}"#]);

    // 有新消息的会话排到前面
    let list = store.list_conversations(10, 0).await.unwrap();
    assert_eq!(list[0].id, first.id);
    assert_eq!(list[1].id, second.id);

    store
        .set_message_status(&recent[0].id, "confirmed", T0 + 6)
        .await
        .unwrap();
    assert_eq!(
        store.get_message(&recent[0].id).await.unwrap().status,
        "confirmed"
    );
    store
        .rename_conversation(&first.id, "改名", T0 + 7)
        .await
        .unwrap();
    assert_eq!(
        store.get_conversation(&first.id).await.unwrap().title,
        "改名"
    );

    assert!(matches!(
        store.append_message("nope", "user", "{}", "done", T0).await,
        Err(StoreError::NotFound { .. })
    ));
    store.delete_conversation(&first.id).await.unwrap();
    assert!(
        store
            .recent_messages(&first.id, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        store.delete_conversation(&first.id).await,
        Err(StoreError::NotFound { .. })
    ));
}

#[tokio::test]
async fn plan_steps_change_state_once() {
    let (store, _dir) = open().await;
    let conversation = store.create_conversation("会话", T0).await.unwrap();
    let (plan, steps) = store
        .create_plan(
            &NewPlan {
                conversation_id: conversation.id.clone(),
                message_id: None,
                tool_call_id: "call_1".into(),
                status: "pending_confirm".into(),
                estimated_jobs: 10,
                steps: vec![step("s1", "[]"), step("s2", r#"["s1"]"#)],
            },
            T0,
        )
        .await
        .unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(
        (steps[0].seq, steps[1].depends_on.as_str()),
        (0, r#"["s1"]"#)
    );
    assert!(steps.iter().all(|s| s.status == "pending"));
    assert_eq!(
        store
            .plans_with_status(&conversation.id, "pending_confirm")
            .await
            .unwrap()
            .len(),
        1
    );

    // 只有当前状态对得上才改
    assert!(
        !store
            .set_plan_status(&plan.id, Some("running"), "succeeded", T0)
            .await
            .unwrap()
    );
    assert!(
        store
            .set_plan_status(&plan.id, Some("pending_confirm"), "running", T0 + 1)
            .await
            .unwrap()
    );
    assert!(
        store
            .plan_for_message("no-such-message")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(store.running_plans().await.unwrap().len(), 1);

    assert!(store.start_step(&steps[0].id, None, T0 + 2).await.unwrap());
    assert!(
        !store.start_step(&steps[0].id, None, T0 + 2).await.unwrap(),
        "不能启动两次"
    );
    store
        .set_step_handle(&steps[0].id, "batch-1", T0 + 3)
        .await
        .unwrap();
    let running = store
        .find_running_step_by_handle("batch-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(running.id, steps[0].id);
    assert_eq!(store.running_steps().await.unwrap().len(), 1);

    assert!(
        store
            .finish_step(
                &steps[0].id,
                "succeeded",
                Some(r#"{"images":7}"#),
                None,
                T0 + 4
            )
            .await
            .unwrap()
    );
    // 重复通知不生效
    assert!(
        !store
            .finish_step(&steps[0].id, "failed", None, Some("x"), T0 + 5)
            .await
            .unwrap()
    );
    let done = store.get_step(&steps[0].id).await.unwrap();
    assert_eq!(
        (done.status.as_str(), done.output.as_deref()),
        ("succeeded", Some(r#"{"images":7}"#))
    );
    assert_eq!(
        (done.started_at, done.finished_at),
        (Some(T0 + 2), Some(T0 + 4))
    );
    assert!(
        store
            .find_running_step_by_handle("batch-1")
            .await
            .unwrap()
            .is_none()
    );

    // 没开始的步骤也能直接结束（跳过、取消）
    assert!(
        store
            .finish_step(&steps[1].id, "skipped", None, Some("前置步骤失败"), T0 + 6)
            .await
            .unwrap()
    );
    assert!(
        store
            .set_plan_status(&plan.id, None, "partial", T0 + 7)
            .await
            .unwrap()
    );
    let finished = store.get_plan(&plan.id).await.unwrap();
    assert_eq!(
        (finished.status.as_str(), finished.finished_at),
        ("partial", Some(T0 + 7))
    );

    // 删除会话，计划和步骤一起删
    store.delete_conversation(&conversation.id).await.unwrap();
    assert!(matches!(
        store.get_plan(&plan.id).await,
        Err(StoreError::NotFound { .. })
    ));
    assert!(store.plan_steps(&plan.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn plans_need_an_existing_conversation_and_unique_step_keys() {
    let (store, _dir) = open().await;
    let plan = |conversation_id: &str, steps: Vec<NewStep>| NewPlan {
        conversation_id: conversation_id.into(),
        message_id: None,
        tool_call_id: "call".into(),
        status: "running".into(),
        estimated_jobs: 0,
        steps,
    };
    assert!(matches!(
        store
            .create_plan(&plan("nope", vec![step("s1", "[]")]), T0)
            .await,
        Err(StoreError::NotFound { .. })
    ));
    let conversation = store.create_conversation("会话", T0).await.unwrap();
    assert!(
        store
            .create_plan(
                &plan(&conversation.id, vec![step("s1", "[]"), step("s1", "[]")]),
                T0
            )
            .await
            .is_err()
    );
    // 失败的事务没留下半个计划
    assert!(
        store
            .plans_with_status(&conversation.id, "running")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn rewinding_drops_later_messages_and_plans() {
    let (store, _dir) = open().await;
    let conversation = store.create_conversation("会话", T0).await.unwrap();
    let say = |role: &'static str, text: &'static str, at: i64| {
        let store = store.clone();
        let id = conversation.id.clone();
        async move {
            store
                .append_message(&id, role, &format!("{{\"text\":\"{text}\"}}"), "done", at)
                .await
                .unwrap()
        }
    };
    say("user", "第一句", T0).await;
    say("assistant", "回答一", T0 + 1).await;
    let second = say("user", "第二句", T0 + 2).await;
    // 同一毫秒、插在第二句后面的消息也要删
    say("assistant", "回答二", T0 + 2).await;
    let (finished, _) = store
        .create_plan(&plan(&conversation.id, vec![step("s1", "[]")]), T0 + 3)
        .await
        .unwrap();
    store
        .add_notification(
            &conversation.id,
            Some(&finished.id),
            "plan_confirm",
            "等确认",
            false,
            T0 + 3,
        )
        .await
        .unwrap();
    let (earlier, _) = store
        .create_plan(&plan(&conversation.id, vec![step("s1", "[]")]), T0 + 1)
        .await
        .unwrap();

    // 助手消息不能当起点
    let assistant = store.recent_messages(&conversation.id, 10).await.unwrap()[1].clone();
    assert!(matches!(
        store.rewind_from(&conversation.id, &assistant.id).await,
        Err(StoreError::Invalid(_))
    ));
    // 之后有计划在执行时拒绝，什么都不删
    assert!(
        store
            .set_plan_status(&finished.id, Some("pending_confirm"), "running", T0 + 4)
            .await
            .unwrap()
    );
    assert!(matches!(
        store.rewind_from(&conversation.id, &second.id).await,
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(
        store
            .recent_messages(&conversation.id, 10)
            .await
            .unwrap()
            .len(),
        4
    );

    assert!(
        store
            .set_plan_status(&finished.id, Some("running"), "succeeded", T0 + 5)
            .await
            .unwrap()
    );
    store
        .rewind_from(&conversation.id, &second.id)
        .await
        .unwrap();
    let left: Vec<String> = store
        .recent_messages(&conversation.id, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.content)
        .collect();
    assert_eq!(left, vec![r#"{"text":"第一句"}"#, r#"{"text":"回答一"}"#]);
    // 之后的计划和它的通知删掉，之前的计划保留
    assert!(store.get_plan(&finished.id).await.is_err());
    assert!(store.get_plan(&earlier.id).await.is_ok());
    assert!(store.list_notifications(10).await.unwrap().is_empty());
}

#[tokio::test]
async fn notifications_track_read_state() {
    let (store, _dir) = open().await;
    let a = store.create_conversation("会话 A", T0).await.unwrap();
    let b = store.create_conversation("会话 B", T0).await.unwrap();
    let (plan_a, _) = store
        .create_plan(&plan(&a.id, vec![step("s1", "[]")]), T0)
        .await
        .unwrap();

    assert!(
        store
            .add_notification(&a.id, Some(&plan_a.id), "plan_confirm", "等确认", false, T0)
            .await
            .unwrap()
    );
    // 同一计划同一种通知只记一次
    assert!(
        !store
            .add_notification(
                &a.id,
                Some(&plan_a.id),
                "plan_confirm",
                "又一次",
                false,
                T0 + 1
            )
            .await
            .unwrap()
    );
    store
        .add_notification(
            &a.id,
            Some(&plan_a.id),
            "plan_finished",
            "完成",
            false,
            T0 + 2,
        )
        .await
        .unwrap();
    // 没有计划的通知不受唯一约束
    store
        .add_notification(&b.id, None, "turn_failed", "出错一", false, T0 + 3)
        .await
        .unwrap();
    store
        .add_notification(&b.id, None, "turn_failed", "出错二", true, T0 + 4)
        .await
        .unwrap();

    let list = store.list_notifications(10).await.unwrap();
    assert_eq!(
        list.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(),
        vec!["出错二", "出错一", "完成", "等确认"]
    );
    assert_eq!(list[0].conversation_title, "会话 B");
    assert_eq!(list[0].read_at, Some(T0 + 4));
    assert_eq!(store.unread_notifications().await.unwrap(), 3);

    assert_eq!(
        store.mark_conversation_read(&a.id, T0 + 5).await.unwrap(),
        2
    );
    assert_eq!(store.unread_notifications().await.unwrap(), 1);
    let ids = vec![list[1].id.clone()];
    assert_eq!(
        store
            .mark_notifications_read(Some(&ids), T0 + 6)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        store.mark_notifications_read(None, T0 + 7).await.unwrap(),
        0
    );
    assert_eq!(store.unread_notifications().await.unwrap(), 0);

    // 删会话时通知一起删
    store.delete_conversation(&b.id).await.unwrap();
    assert_eq!(store.list_notifications(10).await.unwrap().len(), 2);
}
