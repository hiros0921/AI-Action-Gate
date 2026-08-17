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
            weights: self.weights,
            ..Policy::provisional()
        }
    }
}

// ══════════════════════════════════════════════════════════════════
//  閾値案（重みは案W2に固定）
// ══════════════════════════════════════════════════════════════════

/// 閾値案を比べるあいだ固定する重み。**案W2が採用されました。**
///
/// <div class="warning">
///
/// 諏訪の判断（第4段階）:
///
/// > ① 事故の不可逆性が違う。社内の削除はバックアップで戻せる可能性があるが、
/// >   外部AIに渡った情報は取り返せない。型5を見ると、W2だけが1件も自動承認していない。
/// >   「PIIが検出されなかったから通す」は、「検出できなかった」と「無かった」の
/// >   区別と方向が逆。
/// > ② W3・W4は社内の正常業務をHIGHに上げる。決済処理でカード番号をマスクしたら
/// >   業務が止まる。承認画面が正常業務で埋まる。
/// > ③ W2の弱点（社内削除が26点で自動承認）は閾値で直せる。あとから直せる余地が
/// >   W2は大きい。これが決定打。
///
/// </div>
pub const ADOPTED_WEIGHTS: Weights = Weights {
    action: 20,
    destination: 40,
    data_class: 20,
    pii: 20,
};

/// 閾値の案。
pub struct ThresholdProposal {
    pub id: &'static str,
    pub label: &'static str,
    pub stance: &'static str,
    pub thresholds: Thresholds,
}

/// 閾値案。重みは [`ADOPTED_WEIGHTS`] に固定。
///
/// 【重要】諏訪の申し送り（第4段階）:
///
/// > 13〜25の範囲を優先して検討してください。ここなら、社内削除も外部送信も守られます。
/// > 人手は増えますが、このシステムの主張は「安全側から始めて、緩め方を数字で見せる」こと。
/// > 初期値は安全側でいいと思います。
///
/// T1〜T3 がその範囲。T4 は範囲の外に置いた比較用で、
/// 「緩めると何件が自動に落ちるか」を数字で見るためのものです。
pub fn threshold_proposals() -> Vec<ThresholdProposal> {
    vec![
        ThresholdProposal {
            id: "T1",
            label: "帯の下端",
            stance: "型1（社内のread・最大12点）のすぐ上で切る。いちばん安全側",
            thresholds: Thresholds {
                medium_at: 13,
                high_at: 70,
            },
        },
        ThresholdProposal {
            id: "T2",
            label: "帯の中央",
            stance: "13〜25の真ん中。社内の軽い操作に少しだけ余地を残す",
            thresholds: Thresholds {
                medium_at: 20,
                high_at: 70,
            },
        },
        ThresholdProposal {
            id: "T3",
            label: "帯の上端",
            stance: "社内の削除（26点）の直前まで緩める。範囲内でいちばん人手が少ない",
            thresholds: Thresholds {
                medium_at: 25,
                high_at: 70,
            },
        },
        ThresholdProposal {
            id: "T4",
            label: "帯の外（比較用）",
            stance: "推奨範囲の外。緩めると何が自動に落ちるかを見るための対照",
            thresholds: Thresholds {
                medium_at: 40,
                high_at: 70,
            },
        },
        ThresholdProposal {
            id: "T5",
            label: "HIGHを広げる",
            stance: "境目は上端のまま、マスキング案を添える範囲を広げる",
            thresholds: Thresholds {
                medium_at: 25,
                high_at: 55,
            },
        },
    ]
}

impl ThresholdProposal {
    pub fn policy(&self) -> Policy {
        Policy {
            version: format!("threshold-{}", self.id.to_lowercase()),
            label: format!("案{}・{}", self.id, self.label),
            thresholds: self.thresholds,
            weights: ADOPTED_WEIGHTS,
            ..Policy::provisional()
        }
    }
}
