//! 要求の置き場。第3段階はメモリ上だけ（仕様書11章）。
//!
//! <div class="warning">
//!
//! 【重要】trait にしてあるのは、第6段階で DynamoDB に差し替えるためです。
//!
//! ハンドラは [`RequestStore`] しか知りません。差し替えるときに触るのは
//! `main.rs` の1行だけで、判定も画面も変わりません。
//!
//! </div>

use std::collections::HashMap;
use std::sync::RwLock;

use gate_core::mask::MaskPlan;
use gate_core::review::{AuditEntry, Verdict};
use gate_core::score::RiskAssessment;

/// 保存される1件。
#[derive(Debug, Clone)]
pub struct StoredRequest {
    pub id: String,
    pub agent_id: String,
    pub created_at: String,
    pub assessment: RiskAssessment,
    /// 承認画面で見せるための本文。
    ///
    /// 【重要】ここだけが平文を持ちます。監査ログには入りません（仕様書7章）。
    /// 第6段階では、この項目にだけ TTL を付けて自動で消す方針です
    /// （第1段階の修正提案①。承認が終われば要らなくなるため）。
    pub payload: String,
    pub mask_plan: MaskPlan,
    pub state: RequestState,
}

/// 要求がいまどこにいるか。
#[derive(Debug, Clone, PartialEq)]
pub enum RequestState {
    /// 承認待ち。
    Pending,
    /// 判断が下りた。
    Settled {
        verdict: Verdict,
        /// エージェントに返す本文。マスキング承認ならマスキング後。
        returned_payload: Option<String>,
        reviewer: String,
        at: String,
    },
}

impl RequestState {
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }
}

/// 置き場。第6段階で DynamoDB 実装に差し替える。
pub trait RequestStore: Send + Sync {
    fn put(&self, request: StoredRequest);
    fn get(&self, id: &str) -> Option<StoredRequest>;
    /// 承認待ちを古い順に。
    fn pending(&self) -> Vec<StoredRequest>;
    /// 判定済みも含めて新しい順に。閾値シミュレーションが使う。
    fn all(&self) -> Vec<StoredRequest>;
    fn settle(&self, id: &str, state: RequestState) -> Option<StoredRequest>;
    /// 監査ログ。**追記のみ。** 消す関数も直す関数も用意しない。
    fn append_audit(&self, entry: AuditEntry);
    fn audit_log(&self) -> Vec<AuditEntry>;
}

/// メモリ上の実装。
///
/// 【重要】この実装には `update` も `delete` もありません。
/// 第7段階では同じことを IAM で保証しますが、trait の形からして
/// 「消す」を呼べないようにしてあります。
#[derive(Default)]
pub struct InMemoryStore {
    requests: RwLock<HashMap<String, StoredRequest>>,
    order: RwLock<Vec<String>>,
    audit: RwLock<Vec<AuditEntry>>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RequestStore for InMemoryStore {
    fn put(&self, request: StoredRequest) {
        let id = request.id.clone();
        self.requests.write().unwrap().insert(id.clone(), request);
        self.order.write().unwrap().push(id);
    }

    fn get(&self, id: &str) -> Option<StoredRequest> {
        self.requests.read().unwrap().get(id).cloned()
    }

    fn pending(&self) -> Vec<StoredRequest> {
        let requests = self.requests.read().unwrap();
        self.order
            .read()
            .unwrap()
            .iter()
            .filter_map(|id| requests.get(id))
            .filter(|r| r.state.is_pending())
            .cloned()
            .collect()
    }

    fn all(&self) -> Vec<StoredRequest> {
        let requests = self.requests.read().unwrap();
        self.order
            .read()
            .unwrap()
            .iter()
            .rev()
            .filter_map(|id| requests.get(id))
            .cloned()
            .collect()
    }

    fn settle(&self, id: &str, state: RequestState) -> Option<StoredRequest> {
        let mut requests = self.requests.write().unwrap();
        let request = requests.get_mut(id)?;
        if !request.state.is_pending() {
            // 【重要】二重承認を弾く。承認済みの要求をもう一度承認させない。
            return None;
        }
        request.state = state;
        Some(request.clone())
    }

    fn append_audit(&self, entry: AuditEntry) {
        self.audit.write().unwrap().push(entry);
    }

    fn audit_log(&self) -> Vec<AuditEntry> {
        self.audit.read().unwrap().clone()
    }
}
