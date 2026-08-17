//! API の通し試験。サーバを起動せず、ハンドラを直接叩く。
//!
//! DB も外部サービスも要りません。`cargo test` だけで、
//! 三分岐・承認・二重承認の拒否・監査ログ・シミュレーションまで確かめられます。

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use gate_api::routes;
use gate_api::state::AppState;
use gate_api::store::InMemoryStore;
use gate_core::policy::Policy;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

fn app() -> (Router, Arc<AppState>) {
    let state = Arc::new(AppState::new(
        Arc::new(InMemoryStore::new()),
        Policy::provisional(),
    ));
    (routes::router(state.clone()), state)
}

async fn post(app: &Router, path: &str, body: Value) -> (StatusCode, Value) {
    send(
        app,
        Request::post(path).header("content-type", "application/json"),
        body,
    )
    .await
}

async fn post_as(app: &Router, path: &str, approver: &str, body: Value) -> (StatusCode, Value) {
    send(
        app,
        Request::post(path)
            .header("content-type", "application/json")
            .header("x-approver-id", approver),
        body,
    )
    .await
}

async fn get(app: &Router, path: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn send(
    app: &Router,
    builder: axum::http::request::Builder,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

// 【重要】以下の本文はすべて架空です（仕様書9章）。

fn low_request() -> Value {
    json!({
        "agent_id": "agent-001",
        "action": "read",
        "destination": "internal",
        "data_class": "public",
        "payload": { "text": "在庫一覧を確認します。対象は倉庫Aの全SKUです。" }
    })
}

fn medium_request() -> Value {
    json!({
        "agent_id": "agent-001",
        "action": "write",
        "destination": "external",
        "data_class": "internal",
        "payload": { "text": "取引先へ発注書を送信します。連絡先は order@example.com です。" }
    })
}

fn high_request() -> Value {
    json!({
        "agent_id": "agent-001",
        "action": "send_external",
        "destination": "external_ai",
        "data_class": "personal_information",
        "payload": { "text": "山田太郎さん（1980年3月15日生・090-1234-5678）の検査結果について" }
    })
}

#[tokio::test]
async fn low_は人を通さずに自動承認されること() {
    let (app, _) = app();
    let (status, body) = post(&app, "/api/requests", low_request()).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["decision"], "approved");
    assert_eq!(body["risk"], "LOW");

    // 承認待ちには入らない。
    let (_, queue) = get(&app, "/api/queue").await;
    assert_eq!(queue.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn high_は承認待ちになりマスキング案が付くこと() {
    let (app, _) = app();
    let (status, body) = post(&app, "/api/requests", high_request()).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["decision"], "pending");
    assert_eq!(body["risk"], "HIGH");

    // 仕様書2章の応答例と同じ形。
    let preview = body["masked_preview"].as_str().unwrap();
    assert!(
        !preview.contains("山田"),
        "マスキング案に氏名が残っている: {preview}"
    );
    assert!(preview.contains("○"), "伏せ字になっていない: {preview}");

    // 検出は種別と件数のみ。
    let detected = body["detected"].as_array().unwrap();
    assert!(!detected.is_empty());
    assert!(detected.iter().any(|d| d["kind"] == "person_name"));
}

#[tokio::test]
async fn 承認するとエージェントに本文が返ること() {
    let (app, _) = app();
    let (_, submitted) = post(&app, "/api/requests", high_request()).await;
    let id = submitted["request_id"].as_str().unwrap().to_string();

    let (status, _) = post_as(
        &app,
        &format!("/api/requests/{id}/decision"),
        "suwa",
        json!({ "verdict": "approve" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, fetched) = get(&app, &format!("/api/requests/{id}")).await;
    assert_eq!(fetched["status"], "approved");
    assert!(fetched["payload"].as_str().unwrap().contains("山田太郎"));
    assert_eq!(fetched["reviewer"], "suwa");
}

#[tokio::test]
async fn マスキングして承認すると伏せ字のほうが返ること() {
    // 仕様書5章「マスキング後の文字列で承認した場合、エージェントへ返すのはマスキング後のもの」。
    let (app, _) = app();
    let (_, submitted) = post(&app, "/api/requests", high_request()).await;
    let id = submitted["request_id"].as_str().unwrap().to_string();

    post_as(
        &app,
        &format!("/api/requests/{id}/decision"),
        "suwa",
        json!({ "verdict": "approve_masked" }),
    )
    .await;

    let (_, fetched) = get(&app, &format!("/api/requests/{id}")).await;
    assert_eq!(fetched["status"], "approved_masked");
    let returned = fetched["payload"].as_str().unwrap();
    assert!(
        !returned.contains("山田"),
        "元の氏名が返っている: {returned}"
    );
    assert!(returned.contains("○"));
}

#[tokio::test]
async fn 拒否すると本文が返らないこと() {
    let (app, _) = app();
    let (_, submitted) = post(&app, "/api/requests", high_request()).await;
    let id = submitted["request_id"].as_str().unwrap().to_string();

    post_as(
        &app,
        &format!("/api/requests/{id}/decision"),
        "suwa",
        json!({ "verdict": "reject" }),
    )
    .await;

    let (_, fetched) = get(&app, &format!("/api/requests/{id}")).await;
    assert_eq!(fetched["status"], "rejected");
    assert!(fetched["payload"].is_null(), "拒否したのに本文が返っている");
}

#[tokio::test]
async fn 承認者を名乗らないと承認できないこと() {
    // 【重要】認証は実装しないが、「誰が承認したか」が空の記録は作らせない。
    let (app, _) = app();
    let (_, submitted) = post(&app, "/api/requests", high_request()).await;
    let id = submitted["request_id"].as_str().unwrap().to_string();

    let (status, _) = post(
        &app,
        &format!("/api/requests/{id}/decision"),
        json!({ "verdict": "approve" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn 二重承認を弾くこと() {
    let (app, _) = app();
    let (_, submitted) = post(&app, "/api/requests", high_request()).await;
    let id = submitted["request_id"].as_str().unwrap().to_string();
    let path = format!("/api/requests/{id}/decision");

    let (first, _) = post_as(&app, &path, "suwa", json!({ "verdict": "approve" })).await;
    let (second, _) = post_as(&app, &path, "someone", json!({ "verdict": "reject" })).await;

    assert_eq!(first, StatusCode::OK);
    assert_eq!(second, StatusCode::CONFLICT, "二度目の判断が通っている");
}

#[tokio::test]
async fn 監査ログに誰がいつどう判断したかが残ること() {
    let (app, _) = app();
    let (_, submitted) = post(&app, "/api/requests", high_request()).await;
    let id = submitted["request_id"].as_str().unwrap().to_string();
    post_as(
        &app,
        &format!("/api/requests/{id}/decision"),
        "suwa",
        json!({ "verdict": "approve_masked" }),
    )
    .await;

    let (_, log) = get(&app, "/api/audit").await;
    let entry = &log.as_array().unwrap()[0];

    assert_eq!(entry["request_id"], id);
    assert_eq!(entry["reviewer"]["approver_id"], "suwa");
    assert_eq!(entry["verdict"], "approved_with_masking");
    // 【重要】そのときの閾値（仕様書7章）。
    assert_eq!(entry["thresholds"]["medium_at"], 30);
    assert_eq!(entry["thresholds"]["high_at"], 70);
    assert_eq!(entry["policy_version"], "provisional-0");
    assert!(entry["at"].as_str().unwrap().len() > 10);
}

#[tokio::test]
async fn 監査ログに平文が残らないこと() {
    // 仕様書7章。消せないログに個人情報を残さない。
    let (app, _) = app();
    let (_, submitted) = post(&app, "/api/requests", high_request()).await;
    let id = submitted["request_id"].as_str().unwrap().to_string();
    post_as(
        &app,
        &format!("/api/requests/{id}/decision"),
        "suwa",
        json!({ "verdict": "approve" }),
    )
    .await;

    let (_, log) = get(&app, "/api/audit").await;
    let text = log.to_string();
    for leak in ["山田", "太郎", "1980", "5678", "検査結果"] {
        assert!(!text.contains(leak), "監査ログに平文が混ざっている: {leak}");
    }
    assert!(text.contains("person_name"), "種別は残っているべき");
}

#[tokio::test]
async fn 自動承認も監査ログに残ること() {
    // 【重要】人が触ったものだけ残すと、自動で通した分が記録から消える。
    // 閾値を緩めた結果どれだけ通ったかが、あとから追えなくなる。
    let (app, _) = app();
    post(&app, "/api/requests", low_request()).await;

    let (_, log) = get(&app, "/api/audit").await;
    let entry = &log.as_array().unwrap()[0];
    assert_eq!(entry["reviewer"]["kind"], "system");
    assert_eq!(entry["verdict"], "approved");
}

#[tokio::test]
async fn 承認待ち一覧に内訳とプレビューが載ること() {
    let (app, _) = app();
    post(&app, "/api/requests", high_request()).await;

    let (_, queue) = get(&app, "/api/queue").await;
    let item = &queue.as_array().unwrap()[0];

    assert!(
        item["components"].as_array().unwrap().len() >= 4,
        "内訳が無い"
    );
    assert!(item["masked_preview"].as_str().unwrap().contains("○"));
    assert_eq!(item["thresholds"]["medium_at"], 30);
    // なぜその点なのかが読める形であること。
    let why = item["components"][0]["why"].as_str().unwrap();
    assert!(!why.is_empty());
}

#[tokio::test]
async fn 閾値シミュレーションが本文なしで動くこと() {
    // 仕様書3章。保存済みの内訳だけで、過去の要求を再判定する。
    //
    // 【重要】ここで一度、試験の側が間違えた。
    // 「緩めれば自動承認が増える」と決めつけて 95/99 を投げたが、
    // HIGH の要求は素点が100（頭打ち）なので、緩めても動かない。
    // 閾値は 0〜100 の範囲にしか置けないため、100点のものを自動へ回す設定は作れない。
    // これは実装の不具合ではなく、そういう性質。動くのは中間の点にいる要求。
    let (app, _) = app();
    post(&app, "/api/requests", low_request()).await;
    post(&app, "/api/requests", medium_request()).await;
    post(&app, "/api/requests", high_request()).await;

    let (status, loose) = post(
        &app,
        "/api/simulate",
        json!({ "medium_at": 80, "high_at": 99 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(loose["total"], 3);

    // 中間にいた1件が自動承認へ移る。
    assert_eq!(loose["current"]["low"], 1);
    assert_eq!(loose["proposed"]["low"], 2);
    assert_eq!(loose["moving_to_auto"]["count"], 1);
    // 【重要】そのうち PII を含む件数。件数だけ見て緩めると、緩めてはいけないものが混ざる。
    assert_eq!(
        loose["moving_to_auto"]["with_pii"], 1,
        "移った1件はメールアドレスを含む"
    );

    // 人手に回る割合が下がったことが数字で出ること（仕様書3章）。
    assert!(
        loose["proposed"]["human_rate"].as_u64().unwrap()
            < loose["current"]["human_rate"].as_u64().unwrap()
    );
}

#[tokio::test]
async fn 厳しくすると人手に回る件数が増えること() {
    let (app, _) = app();
    post(&app, "/api/requests", low_request()).await;
    post(&app, "/api/requests", medium_request()).await;

    let (_, strict) = post(
        &app,
        "/api/simulate",
        json!({ "medium_at": 5, "high_at": 10 }),
    )
    .await;
    assert_eq!(strict["proposed"]["low"], 0, "全部が人手に回るはず");
    assert_eq!(strict["proposed"]["human_rate"], 100);
    assert_eq!(strict["moving_to_auto"]["count"], 0);
}

#[tokio::test]
async fn 逆さまの閾値ではシミュレーションできないこと() {
    let (app, _) = app();
    let (status, body) = post(
        &app,
        "/api/simulate",
        json!({ "medium_at": 90, "high_at": 20 }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["message"].as_str().unwrap().contains("閾値が逆"));
}

#[tokio::test]
async fn 形式が壊れた要求を弾くこと() {
    let (app, _) = app();
    let (status, body) = post(
        &app,
        "/api/requests",
        json!({
            "agent_id": "",
            "action": "read",
            "destination": "internal",
            "payload": { "text": "" }
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    let details = body["details"].as_array().unwrap();
    assert_eq!(details.len(), 2, "問題は全部返すこと: {details:?}");
}

#[tokio::test]
async fn データ区分の申告が無い要求を公開扱いにしないこと() {
    // 付け忘れるほど安全に判定される、という逆の挙動を防ぐ。
    let (app, _) = app();
    let without = json!({
        "agent_id": "agent-001",
        "action": "read",
        "destination": "internal",
        "payload": { "text": "在庫を確認します" }
    });
    let mut with_public = without.clone();
    with_public["data_class"] = json!("public");

    let (_, a) = post(&app, "/api/requests", without).await;
    let (_, b) = post(&app, "/api/requests", with_public).await;

    assert!(
        a["score"].as_u64().unwrap() > b["score"].as_u64().unwrap(),
        "申告なしが公開と同じ点になっている"
    );
}

#[tokio::test]
async fn 設定の版が画面から見えること() {
    // 第4段階までは暫定であることが分かるようにしておく。
    let (app, _) = app();
    let (_, body) = get(&app, "/api/policy").await;
    assert_eq!(body["version"], "provisional-0");
    assert_eq!(body["adopted"], false);
}
