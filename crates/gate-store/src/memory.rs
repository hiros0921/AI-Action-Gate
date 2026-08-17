//! 要求の置き場。メモリ版（第3段階）と DynamoDB 版（第6段階）。
//!
//! <div class="warning">
//!
//! 【重要】trait にしてあるので、ハンドラは置き場の中身を知りません。
//!
//! メモリと DynamoDB を差し替えるときに触るのは `main.rs` の1行だけで、
//! 判定も画面も変わりません。第3段階で先に trait にしておいたので、
//! 第6段階では実装を1つ足すだけで済みました。
//!
//! </div>

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

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
    /// <div class="warning">
    ///
    /// 【重要】DynamoDB 版では、これは<b>別テーブル</b>に入ります（諏訪の指示・第6段階）。
    ///
    /// TTL はアイテム単位で丸ごと消すので、判定結果と同じアイテムに置くと
    /// 内訳も閾値も一緒に消えます。それでは「平文を消したあとでも
    /// シミュレーションできる」という利点が失われます。
    ///
    /// 期限切れで読めなくなった場合は空文字になります。
    /// 「消えた」ことは [`StoredRequest::payload_available`] で分かります。
    ///
    /// </div>
    pub payload: String,
    /// 平文がまだ読めるか。期限切れなら false。
    pub payload_available: bool,
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
#[async_trait]
pub trait RequestStore: Send + Sync {
    async fn put(&self, request: StoredRequest);
    async fn get(&self, id: &str) -> Option<StoredRequest>;
    /// 承認待ちを古い順に。
    async fn pending(&self) -> Vec<StoredRequest>;
    /// 判定済みも含めて新しい順に。閾値シミュレーションが使う。
    async fn all(&self) -> Vec<StoredRequest>;
    async fn settle(&self, id: &str, state: RequestState) -> Option<StoredRequest>;
    /// 監査ログ。**追記のみ。** 消す関数も直す関数も用意しない。
    async fn append_audit(&self, entry: AuditEntry);
    async fn audit_log(&self) -> Vec<AuditEntry>;

    /// 状態が変わるたびに増える番号。
    ///
    /// 【重要】ポーリングを安くするためのものです（第5段階）。
    /// 画面は数秒ごとに一覧を取りに来ますが、承認待ちは1日に数件しか増えません。
    /// この番号を ETag にして、変わっていなければ 304 を返せば、
    /// 本文を作らず、JSON にもせずに済みます。
    async fn version(&self) -> u64;
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
    /// 状態が変わった回数。ETag に使う。
    version: AtomicU64,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl RequestStore for InMemoryStore {
    async fn put(&self, request: StoredRequest) {
        let id = request.id.clone();
        self.requests.write().unwrap().insert(id.clone(), request);
        self.order.write().unwrap().push(id);
        self.version.fetch_add(1, Ordering::Relaxed);
    }

    async fn get(&self, id: &str) -> Option<StoredRequest> {
        self.requests.read().unwrap().get(id).cloned()
    }

    async fn pending(&self) -> Vec<StoredRequest> {
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

    async fn all(&self) -> Vec<StoredRequest> {
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

    async fn settle(&self, id: &str, state: RequestState) -> Option<StoredRequest> {
        let mut requests = self.requests.write().unwrap();
        let request = requests.get_mut(id)?;
        if !request.state.is_pending() {
            // 【重要】二重承認を弾く。承認済みの要求をもう一度承認させない。
            return None;
        }
        request.state = state;
        self.version.fetch_add(1, Ordering::Relaxed);
        Some(request.clone())
    }

    async fn append_audit(&self, entry: AuditEntry) {
        self.audit.write().unwrap().push(entry);
    }

    async fn audit_log(&self) -> Vec<AuditEntry> {
        self.audit.read().unwrap().clone()
    }

    async fn version(&self) -> u64 {
        self.version.load(Ordering::Relaxed)
    }
}
