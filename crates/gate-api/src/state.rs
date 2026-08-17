//! アプリの状態。
//!
//! 【重要】時刻と ID の採番はここに集めてあります。
//! `gate_core` の側では時刻を取りません。取ると、判定が「いつ動かしたか」で
//! 変わるようになり、閾値シミュレーションが成立しなくなります。

use std::sync::{Arc, RwLock};

use gate_core::policy::Policy;

use crate::store::RequestStore;

pub struct AppState {
    pub store: Arc<dyn RequestStore>,
    policy: RwLock<Policy>,
}

impl AppState {
    pub fn new(store: Arc<dyn RequestStore>, policy: Policy) -> Self {
        Self {
            store,
            policy: RwLock::new(policy),
        }
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
