//! アプリの状態。
//!
//! 【重要】時刻と ID の採番はここに集めてあります。
//! `gate_core` の側では時刻を取りません。取ると、判定が「いつ動かしたか」で
//! 変わるようになり、閾値シミュレーションが成立しなくなります。

use std::sync::{Arc, RwLock};

use gate_core::policy::Policy;

use gate_store::RequestStore;

pub struct AppState {
    pub store: Arc<dyn RequestStore>,
    policy: RwLock<Policy>,
    /// 画面が一覧を取りに来る間隔（ミリ秒）。
    ///
    /// <div class="warning">
    ///
    /// 【重要】ハードコードしない（諏訪の指示・第5段階）:
    ///
    /// > 304 でも Lambda の呼び出し回数はカウントされるので、間隔がそのまま費用に効きます。
    /// > ハードコードすると、費用を測ったあとに調整できません。
    ///
    /// 環境変数 `GATE_POLL_MS` で変えられます。既定は3秒。
    /// 第7段階で月額を見積もったあと、ここを動かして調整できます。
    ///
    /// </div>
    poll_interval_ms: u64,
    /// 置き場の名前。画面と /api/health に出す。
    store_kind: &'static str,
}

impl AppState {
    pub fn new(store: Arc<dyn RequestStore>, policy: Policy) -> Self {
        Self::with_poll_interval(store, policy, Self::poll_interval_from_env())
    }

    pub fn with_poll_interval(
        store: Arc<dyn RequestStore>,
        policy: Policy,
        poll_interval_ms: u64,
    ) -> Self {
        Self {
            store_kind: "in-memory",
            store,
            policy: RwLock::new(policy),
            poll_interval_ms,
        }
    }

    /// 置き場の名前を差し替える（DynamoDB 版で使う）。
    pub fn with_store_kind(mut self, kind: &'static str) -> Self {
        self.store_kind = kind;
        self
    }

    pub fn store_kind(&self) -> &'static str {
        self.store_kind
    }

    /// 環境変数から。読めない値なら既定に落とす。
    ///
    /// 【重要】1秒未満にはしない。費用が線形に増えるだけで、
    /// 承認待ちが1日に数件という前提では体感が変わらないため。
    fn poll_interval_from_env() -> u64 {
        std::env::var("GATE_POLL_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(|v| v.max(1_000))
            .unwrap_or(3_000)
    }

    pub fn poll_interval_ms(&self) -> u64 {
        self.poll_interval_ms
    }

    pub fn policy(&self) -> Policy {
        self.policy.read().unwrap().clone()
    }

    pub fn set_policy(&self, policy: Policy) {
        *self.policy.write().unwrap() = policy;
    }

    /// いまの時刻。ISO 8601（UTC）。
    pub fn now(&self) -> String {
        chrono::Utc::now().to_rfc3339()
    }

    /// 要求のID。仕様書2章の例に合わせて `req-` + 8桁。
    pub fn next_request_id(&self) -> String {
        let uuid = uuid::Uuid::new_v4().simple().to_string();
        format!("req-{}", &uuid[..8])
    }
}
