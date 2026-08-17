//! HTTP の口。
//!
//! ここは**受け取って渡すだけ**にしてあります。危険かどうかの判断も、
//! 伏せ字の作り方も、`gate_core` の側にあります。

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use gate_core::action::ActionRequest;
use gate_core::detect::{self, DetectConfig};
use gate_core::mask;
use gate_core::policy::{Decision, Policy, Thresholds};
use gate_core::review::{AuditEntry, Reviewer, Verdict};
use gate_core::score::{self, RiskAssessment};
use serde::{Deserialize, Serialize};

use crate::approver::Approver;
use crate::state::AppState;
use crate::store::{RequestState, StoredRequest};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/requests", post(submit))
        .route("/api/requests/{id}", get(fetch))
        .route("/api/requests/{id}/decision", post(decide))
        .route("/api/queue", get(queue))
        .route("/api/audit", get(audit))
        .route("/api/policy", get(policy))
        .route("/api/simulate", post(simulate))
        .route("/api/health", get(health))
        .with_state(state)
}

// ── エージェントからの入口 ─────────────────────────────────

/// エージェントへの応答（仕様書2章の応答例）。
#[derive(Debug, Serialize)]
pub struct SubmitResponse {
    pub request_id: String,
    /// `approved` / `pending` / `rejected`。
    pub decision: &'static str,
    pub risk: Decision,
    pub score: u8,
    pub detected: Vec<gate_core::detect::KindCount>,
    /// HIGH のときに添えるマスキング案（仕様書2章）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub masked_preview: Option<String>,
    /// 走査を終えているか。false なら「PIIが無い」とは言えない。
    pub conclusive: bool,
    /// 走査できなかったために人手へ回したか。
    pub raised_by_uncertainty: bool,
}

async fn submit(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ActionRequest>,
) -> Result<Json<SubmitResponse>, (StatusCode, Json<ErrorBody>)> {
    let cfg = DetectConfig::default();

    // ① 形式の検証。
    if let Err(errors) = request.validate(cfg.payload_limit) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                message: "要求の形式が正しくありません".to_string(),
                details: errors.iter().map(|e| e.message()).collect(),
            }),
        ));
    }

    let policy = state.policy();

    // ② 検出 → ③ 配点。どちらも gate_core の純粋関数。
    let scan = detect::scan(&request.payload.text, &cfg);
    let assessment = score::assess(&request, &scan, &policy);
    let plan = mask::plan(&request.payload.text, &scan);
    let masked = plan.apply(&request.payload.text);

    let id = state.next_request_id();
    let now = state.now();

    // ④ 三分岐。LOW だけが人を通らない。
    let (state_after, decision_word) = if assessment.decision == Decision::Low {
        (
            RequestState::Settled {
                verdict: Verdict::Approved,
                returned_payload: Some(request.payload.text.clone()),
                reviewer: Reviewer::System.describe(),
                at: now.clone(),
            },
            "approved",
        )
    } else {
        (RequestState::Pending, "pending")
    };

    let stored = StoredRequest {
        id: id.clone(),
        agent_id: request.agent_id.clone(),
        created_at: now.clone(),
        assessment: assessment.clone(),
        payload: request.payload.text.clone(),
        mask_plan: plan,
        state: state_after,
    };
    state.store.put(stored);

    // 自動承認も監査ログに残す。
    //
    // 【重要】「人が承認したもの」だけを残すと、自動で通した分が記録に出てきません。
    // 閾値を緩めた結果どれだけ自動で通ったかが、あとから追えなくなります。
    if assessment.decision == Decision::Low {
        state.store.append_audit(AuditEntry::new(
            &id,
            &now,
            Reviewer::System,
            Verdict::Approved,
            &assessment,
        ));
    }

    Ok(Json(SubmitResponse {
        request_id: id,
        decision: decision_word,
        risk: assessment.decision,
        score: assessment.score,
        detected: assessment.detected.clone(),
        // マスキング案は HIGH のときに添える（仕様書2章）。
        masked_preview: (assessment.decision == Decision::High).then_some(masked),
        conclusive: assessment.conclusive,
        raised_by_uncertainty: assessment.raised_by_uncertainty,
    }))
}

/// エージェントが `request_id` で結果を取りに来る（仕様書6章）。
#[derive(Debug, Serialize)]
pub struct FetchResponse {
    pub request_id: String,
    pub status: &'static str,
    pub risk: Decision,
    pub score: u8,
    pub detected: Vec<gate_core::detect::KindCount>,
    /// 承認された場合に、実行してよい本文。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<String>,
}

async fn fetch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<FetchResponse>, (StatusCode, Json<ErrorBody>)> {
    let stored = state.store.get(&id).ok_or_else(|| not_found(&id))?;
    let a = &stored.assessment;

    let body = match &stored.state {
        RequestState::Pending => FetchResponse {
            request_id: stored.id.clone(),
            status: "pending",
            risk: a.decision,
            score: a.score,
            detected: a.detected.clone(),
            payload: None,
            reviewer: None,
            decided_at: None,
        },
        RequestState::Settled {
            verdict,
            returned_payload,
            reviewer,
            at,
        } => FetchResponse {
            request_id: stored.id.clone(),
            status: match verdict {
                Verdict::Rejected => "rejected",
                Verdict::Approved => "approved",
                Verdict::ApprovedWithMasking => "approved_masked",
            },
            risk: a.decision,
            score: a.score,
            detected: a.detected.clone(),
            payload: returned_payload.clone(),
            reviewer: Some(reviewer.clone()),
            decided_at: Some(at.clone()),
        },
    };
    Ok(Json(body))
}

// ── 承認ダッシュボードの口 ─────────────────────────────────

/// 承認待ちの1件（一覧・詳細で共通）。
#[derive(Debug, Serialize)]
pub struct QueueItem {
    pub request_id: String,
    pub agent_id: String,
    pub created_at: String,
    pub risk: Decision,
    pub score: u8,
    pub thresholds: Thresholds,
    pub policy_version: String,
    pub components: Vec<score::Component>,
    pub detected: Vec<gate_core::detect::KindCount>,
    pub clamped: bool,
    pub conclusive: bool,
    pub raised_by_uncertainty: bool,
    /// 承認画面に見せる本文。
    pub payload: String,
    /// 伏せ字にした場合のプレビュー。
    pub masked_preview: String,
}

async fn queue(State(state): State<Arc<AppState>>) -> Json<Vec<QueueItem>> {
    Json(
        state
            .store
            .pending()
            .into_iter()
            .map(|r| {
                let masked = r.mask_plan.apply(&r.payload);
                QueueItem {
                    request_id: r.id,
                    agent_id: r.agent_id,
                    created_at: r.created_at,
                    risk: r.assessment.decision,
                    score: r.assessment.score,
                    thresholds: r.assessment.thresholds,
                    policy_version: r.assessment.policy_version.clone(),
                    components: r.assessment.components.clone(),
                    detected: r.assessment.detected.clone(),
                    clamped: r.assessment.clamped,
                    conclusive: r.assessment.conclusive,
                    raised_by_uncertainty: r.assessment.raised_by_uncertainty,
                    payload: r.payload,
                    masked_preview: masked,
                }
            })
            .collect(),
    )
}

#[derive(Debug, Deserialize)]
pub struct DecisionBody {
    /// `approve` / `approve_masked` / `reject`
    pub verdict: String,
}

#[derive(Debug, Serialize)]
pub struct DecisionResponse {
    pub request_id: String,
    pub verdict: Verdict,
    pub reviewer: String,
    pub at: String,
}

async fn decide(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Approver(reviewer): Approver,
    Json(body): Json<DecisionBody>,
) -> Result<Json<DecisionResponse>, (StatusCode, Json<ErrorBody>)> {
    let verdict = match body.verdict.as_str() {
        "approve" => Verdict::Approved,
        "approve_masked" => Verdict::ApprovedWithMasking,
        "reject" => Verdict::Rejected,
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    message: format!("知らない判断です: {other}"),
                    details: vec!["approve / approve_masked / reject のいずれか".to_string()],
                }),
            ));
        }
    };

    let stored = state.store.get(&id).ok_or_else(|| not_found(&id))?;
    let now = state.now();

    // マスキング承認なら、返すのは伏せ字にしたほう（仕様書5章）。
    let returned = match verdict {
        Verdict::Approved => Some(stored.payload.clone()),
        Verdict::ApprovedWithMasking => Some(stored.mask_plan.apply(&stored.payload)),
        Verdict::Rejected => None,
    };

    let settled = state.store.settle(
        &id,
        RequestState::Settled {
            verdict,
            returned_payload: returned,
            reviewer: reviewer.describe(),
            at: now.clone(),
        },
    );

    if settled.is_none() {
        // 【重要】承認済みのものを二度承認させない。
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorBody {
                message: "この要求はすでに判断済みです".to_string(),
                details: vec![],
            }),
        ));
    }

    state.store.append_audit(AuditEntry::new(
        &id,
        &now,
        reviewer.clone(),
        verdict,
        &stored.assessment,
    ));

    Ok(Json(DecisionResponse {
        request_id: id,
        verdict,
        reviewer: reviewer.describe(),
        at: now,
    }))
}

async fn audit(State(state): State<Arc<AppState>>) -> Json<Vec<AuditEntry>> {
    Json(state.store.audit_log())
}

#[derive(Debug, Serialize)]
pub struct PolicyResponse {
    pub version: String,
    pub label: String,
    pub thresholds: Thresholds,
    pub weights: gate_core::policy::Weights,
    /// 採用済みか。第4段階まで false。
    pub adopted: bool,
}

async fn policy(State(state): State<Arc<AppState>>) -> Json<PolicyResponse> {
    let p = state.policy();
    Json(PolicyResponse {
        adopted: p.version.starts_with("adopted"),
        version: p.version.clone(),
        label: p.label.clone(),
        thresholds: p.thresholds,
        weights: p.weights,
    })
}

// ── 閾値シミュレーション（仕様書3章） ─────────────────────

#[derive(Debug, Deserialize)]
pub struct SimulateBody {
    pub medium_at: u8,
    pub high_at: u8,
}

#[derive(Debug, Serialize)]
pub struct SimulateResponse {
    pub current: Tally,
    pub proposed: Tally,
    pub thresholds: Thresholds,
    /// 自動承認へ移る件数と、そのうち PII を含む件数。
    pub moving_to_auto: Moved,
    pub total: usize,
}

#[derive(Debug, Serialize, Default)]
pub struct Tally {
    pub low: usize,
    pub medium: usize,
    pub high: usize,
    /// 人手に回る割合（％）。
    pub human_rate: u8,
}

#[derive(Debug, Serialize, Default)]
pub struct Moved {
    pub count: usize,
    /// 【重要】そのうち PII を含むもの。件数だけ見て緩めると、緩めてはいけないものが混ざる。
    pub with_pii: usize,
}

async fn simulate(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SimulateBody>,
) -> Result<Json<SimulateResponse>, (StatusCode, Json<ErrorBody>)> {
    let thresholds = Thresholds::new(body.medium_at, body.high_at).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                message: format!(
                    "閾値が逆です（MEDIUM {} > HIGH {}）。この設定では MEDIUM が出ません",
                    body.medium_at, body.high_at
                ),
                details: vec![],
            }),
        )
    })?;

    let policy = state.policy();
    let requests = state.store.all();

    let mut current = Tally::default();
    let mut proposed = Tally::default();
    let mut moved = Moved::default();

    for r in &requests {
        let a: &RiskAssessment = &r.assessment;
        // 【重要】本文は読み直しません。保存済みの内訳だけで再判定します。
        let before = score::redecide(a, a.thresholds, &policy);
        let after = score::redecide(a, thresholds, &policy);
        count_into(&mut current, before);
        count_into(&mut proposed, after);
        if before.needs_human() && !after.needs_human() {
            moved.count += 1;
            if !a.detected.is_empty() {
                moved.with_pii += 1;
            }
        }
    }

    finish(&mut current, requests.len());
    finish(&mut proposed, requests.len());

    Ok(Json(SimulateResponse {
        current,
        proposed,
        thresholds,
        moving_to_auto: moved,
        total: requests.len(),
    }))
}

fn count_into(tally: &mut Tally, decision: Decision) {
    match decision {
        Decision::Low => tally.low += 1,
        Decision::Medium => tally.medium += 1,
        Decision::High => tally.high += 1,
    }
}

fn finish(tally: &mut Tally, total: usize) {
    tally.human_rate = if total == 0 {
        0
    } else {
        (((tally.medium + tally.high) as f32 / total as f32) * 100.0).round() as u8
    };
}

// ── 雑務 ────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub message: String,
    pub details: Vec<String>,
}

fn not_found(id: &str) -> (StatusCode, Json<ErrorBody>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorBody {
            message: format!("その要求はありません: {id}"),
            details: vec![],
        }),
    )
}

async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "policy": state.policy().version,
        "store": "in-memory",
    }))
}

/// 設定を差し替えるための入口（試験で使う）。
pub fn with_policy(state: &AppState, policy: Policy) {
    state.set_policy(policy);
}
