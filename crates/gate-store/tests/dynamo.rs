//! DynamoDB Local に対する試験。
//!
//! ```bash
//! docker compose up -d
//! GATE_DYNAMO_ENDPOINT=http://localhost:18000 AWS_ACCESS_KEY_ID=local \
//!   AWS_SECRET_ACCESS_KEY=local AWS_REGION=ap-northeast-1 \
//!   cargo test -p gate-store -- --ignored --test-threads=1
//! ```
//!
//! <div class="warning">
//!
//! 【重要】`#[ignore]` を付けてあります。
//!
//! `cargo test` に Docker を要求すると、判定コアの試験まで Docker 無しでは
//! 走らなくなります。「サーバもDBも立てずに全部通る」を守るため、
//! DB が要る試験だけは明示的に呼び出す形にしました。
//!
//! </div>

use gate_core::action::{ActionKind, ActionRequest, DataClass, Destination, Payload};
use gate_core::detect::{DetectConfig, scan};
use gate_core::policy::Policy;
use gate_core::review::Verdict;
use gate_core::score::assess;
use gate_store::dynamo::DynamoStore;
use gate_store::{PayloadStore, RequestState, RequestStore, StoredRequest};

/// 架空の要求を1件作る（仕様書9章。実在の個人情報は含めない）。
fn sample(id: &str, created_at: &str) -> StoredRequest {
    let text = "山田太郎さん（1980年3月15日生・090-1234-5678）の検査結果について";
    let request = ActionRequest {
        agent_id: "agent-001".into(),
        action: ActionKind::SendExternal,
        destination: Destination::ExternalAi,
        data_class: DataClass::PersonalInformation,
        payload: Payload { text: text.into() },
    };
    let s = scan(text, &DetectConfig::default());
    StoredRequest {
        id: id.to_string(),
        agent_id: request.agent_id.clone(),
        created_at: created_at.to_string(),
        assessment: assess(&request, &s, &Policy::adopted()),
        payload: text.to_string(),
        payload_available: true,
        mask_plan: gate_core::mask::plan(text, &s),
        state: RequestState::Pending,
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

async fn store() -> DynamoStore {
    let client = gate_store::connect().await;
    gate_store::ensure_tables(&client)
        .await
        .expect("テーブルを用意できません。docker compose up -d は済んでいますか");
    DynamoStore::new(client)
}

#[tokio::test]
#[ignore = "DynamoDB Local が要る"]
async fn 期限を過ぎた平文は読めないこと() {
    // 【重要】諏訪の指示（第6段階）:
    //   「DynamoDBのTTLは、期限を過ぎてから通常48時間以内に削除される。
    //     削除されるまで Query や Scan の結果に出続ける。
    //     なので、アプリ側で期限切れをフィルタしてください」
    //
    // この試験は、その「削除されるまでの間」を再現します。
    // アイテムは残したまま、期限だけを過去にします。
    let client = gate_store::connect().await;
    gate_store::ensure_tables(&client).await.unwrap();
    let payloads = PayloadStore::new(client);
    let id = format!("test-expired-{}", now());

    payloads
        .put_payload(&id, "山田太郎さんの検査結果")
        .await
        .unwrap();
    assert!(
        payloads.get_payload(&id, now()).await.unwrap().is_some(),
        "置いた直後に読めない"
    );

    // 1時間前に期限が切れたことにする。DynamoDB はまだ消していない。
    payloads
        .schedule_deletion(&id, now() - 3_600)
        .await
        .unwrap();

    assert!(
        payloads.get_payload(&id, now()).await.unwrap().is_none(),
        "期限を過ぎた平文が読めてしまう（TTLの削除待ちで残っている）"
    );
}

#[tokio::test]
#[ignore = "DynamoDB Local が要る"]
async fn 平文が消えても判定結果は残ること() {
    // 【重要】諏訪の指示（第6段階）:
    //   「平文と判定結果が同じアイテムだと、内訳も閾値も一緒に消えます。
    //     『平文を消したあとでもシミュレーションできる』という利点が消えます」
    let store = store().await;
    let id = format!("test-survive-{}", now());
    let request = sample(&id, &chrono::Utc::now().to_rfc3339());
    let expected_score = request.assessment.score;
    store.put(request).await;

    // 平文の期限を過去にする（TTL の削除を待たずに、読めない状態を作る）。
    let client = gate_store::connect().await;
    PayloadStore::new(client)
        .schedule_deletion(&id, now() - 3_600)
        .await
        .unwrap();

    let after = store.get(&id).await.expect("判定結果まで消えている");
    assert!(!after.payload_available, "平文が読めてしまう");
    assert_eq!(after.payload, "", "平文が残っている");
    // 内訳と閾値は残っている。これがあるからシミュレーションできる。
    assert_eq!(after.assessment.score, expected_score);
    assert!(!after.assessment.components.is_empty(), "内訳が消えている");
    assert_eq!(after.assessment.thresholds, Policy::adopted().thresholds);
}

#[tokio::test]
#[ignore = "DynamoDB Local が要る"]
async fn 放置された承認待ちを期限切れとして閉じること() {
    // 【重要】諏訪の指示（第6段階）:
    //   「『承認完了後24時間』だと、承認待ちのまま放置された平文が永久に残ります。
    //     一定期間で『期限切れ』として閉じて、そこからTTLを開始する形にしてください」
    let store = store().await;
    let id = format!("test-abandoned-{}", now());
    // 30日前に受け付けたまま、誰も判断していない要求。
    let old = chrono::Utc::now() - chrono::Duration::days(30);
    store.put(sample(&id, &old.to_rfc3339())).await;

    let closed = store
        .expire_abandoned(7 * 24 * 3_600, &chrono::Utc::now().to_rfc3339())
        .await;
    assert!(closed >= 1, "放置された要求が閉じられていない");

    let after = store.get(&id).await.unwrap();
    match after.state {
        RequestState::Settled { verdict, .. } => {
            assert_eq!(verdict, Verdict::Expired, "人の拒否として閉じている");
        }
        RequestState::Pending => panic!("承認待ちのまま残っている"),
    }

    // 【重要】判断しなかったことも記録。監査ログに残る。
    let log = store.audit_log().await;
    let entry = log
        .iter()
        .find(|e| e.request_id == id)
        .expect("期限切れが監査ログに残っていない");
    assert_eq!(entry.verdict, Verdict::Expired);
    assert!(!entry.reviewer.is_human(), "人が判断したことになっている");
    // そのときの閾値も残る。
    assert_eq!(entry.thresholds, Policy::adopted().thresholds);
}

#[tokio::test]
#[ignore = "DynamoDB Local が要る"]
async fn 新しい要求はまだ閉じないこと() {
    // 掃除が効きすぎると、承認する前に閉じてしまう。
    let store = store().await;
    let id = format!("test-fresh-{}", now());
    store
        .put(sample(&id, &chrono::Utc::now().to_rfc3339()))
        .await;

    store
        .expire_abandoned(7 * 24 * 3_600, &chrono::Utc::now().to_rfc3339())
        .await;

    let after = store.get(&id).await.unwrap();
    assert!(
        after.state.is_pending(),
        "受け付けたばかりの要求が閉じられた"
    );
}

#[tokio::test]
#[ignore = "DynamoDB Local が要る"]
async fn 二重承認を条件式で弾くこと() {
    // 【重要】アプリの if 文ではなく DynamoDB の条件式で弾く。
    // 同時に2人が押しても、通るのは片方だけ。
    let store = store().await;
    let id = format!("test-double-{}", now());
    store
        .put(sample(&id, &chrono::Utc::now().to_rfc3339()))
        .await;

    let settle = |verdict, who: &str| RequestState::Settled {
        verdict,
        returned_payload: None,
        reviewer: who.to_string(),
        at: chrono::Utc::now().to_rfc3339(),
    };

    assert!(
        store
            .settle(&id, settle(Verdict::Approved, "suwa"))
            .await
            .is_some()
    );
    assert!(
        store
            .settle(&id, settle(Verdict::Rejected, "other"))
            .await
            .is_none(),
        "二度目の判断が通った"
    );
}

#[tokio::test]
#[ignore = "DynamoDB Local が要る"]
async fn 監査ログを上書きできないこと() {
    // 【重要】追記のみ（仕様書7章）。同じキーへの put_item も通さない。
    // 第7段階では IAM でも縛るが、アプリ側でも塞いでおく。
    let store = store().await;
    let id = format!("test-audit-{}", now());
    let request = sample(&id, &chrono::Utc::now().to_rfc3339());
    let assessment = request.assessment.clone();
    let at = "2026-08-18T00:00:00Z";

    let entry = |who: &str| {
        gate_core::review::AuditEntry::new(
            &id,
            at,
            gate_core::review::human(who).unwrap(),
            Verdict::Approved,
            &assessment,
        )
    };

    store.append_audit(entry("suwa")).await;
    store.append_audit(entry("someone-else")).await; // 同じキー。通ってはいけない

    let log = store.audit_log().await;
    let mine: Vec<_> = log.iter().filter(|e| e.request_id == id).collect();
    assert_eq!(mine.len(), 1, "同じキーの監査ログが増えている");
    assert_eq!(
        mine[0].reviewer.describe(),
        "suwa",
        "あとから来たほうで上書きされている"
    );
}
