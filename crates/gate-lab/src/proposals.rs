//! 重みと閾値の案。
//!
//! <div class="warning">
//!
//! 【重要】重みと閾値を、同時に変えません（諏訪の指示・第4段階）。
//!
//! > 同時に変えると、判定の差がどちらから来たのか分からなくなります。
//! > まず重み案を比較して1つ選び、そのうえで閾値案を比較する。この順です。
//!
//! だから [`weight_proposals`] は**閾値を固定**し、[`threshold_proposals`] は
//! **重みを固定**します。固定する側は、選ばれるまで暫定値のままです。
//!
//! </div>
//!
//! <div class="warning">
//!
//! 【重要】ここにある数値は**すべて案**です。採用されたものはありません。
//! 仕様書4章「閾値と重みは諏訪が決める。AIが独自に決定しないこと」。
//!
//! </div>

use gate_core::policy::{Policy, Thresholds, Weights};

/// 重み案を比べるあいだ固定する閾値。
///
/// 暫定値（30/70）をそのまま使います。**この値の良し悪しは、次の段階で見ます。**
/// ここで一緒に動かすと、差がどちらから来たのか分からなくなります。
pub const FIXED_THRESHOLDS: Thresholds = Thresholds {
    medium_at: 30,
    high_at: 70,
};

/// 重みの案。
pub struct WeightProposal {
    pub id: &'static str,
    pub label: &'static str,
    pub stance: &'static str,
    pub weights: Weights,
}

/// 重み案4つ。閾値は [`FIXED_THRESHOLDS`] に固定。
pub fn weight_proposals() -> Vec<WeightProposal> {
    vec![
        WeightProposal {
            id: "W1",
            label: "均等",
            stance: "4つの要素を同じだけ見る。どれかに寄せる根拠が無いなら、ここが出発点",
            weights: Weights {
                action: 25,
                destination: 25,
                data_class: 25,
                pii: 25,
            },
        },
        WeightProposal {
            id: "W2",
            label: "送信先を重く",
            stance: "「どこへ出るか」が事故の分かれ目。社内に留まるなら中身が何でも取り返しがつく",
            weights: Weights {
                action: 20,
                destination: 40,
                data_class: 20,
                pii: 20,
            },
        },
        WeightProposal {
            id: "W3",
            label: "中身を重く",
            stance: "個人情報そのものを重く見る。データ区分とPIIで判断し、経路は補助に回す",
            weights: Weights {
                action: 15,
                destination: 20,
                data_class: 35,
                pii: 30,
            },
        },
        WeightProposal {
            id: "W4",
            label: "検出を重く",
            stance: "申告（データ区分）は当てにしない。実際に検出されたPIIで判断する",
            weights: Weights {
                action: 15,
                destination: 25,
                data_class: 15,
                pii: 45,
            },
        },
    ]
}

impl WeightProposal {
    /// この案の設定を作る。閾値は固定。
    pub fn policy(&self) -> Policy {
        Policy {
            version: format!("weight-{}", self.id.to_lowercase()),
            label: format!("案{}・{}", self.id, self.label),
            thresholds: FIXED_THRESHOLDS,
            weights: self.weights.clone(),
            ..Policy::provisional()
        }
    }
}
