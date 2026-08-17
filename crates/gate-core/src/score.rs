//! リスクスコアの計算。
//!
//! <div class="warning">
//!
//! 【重要】この関数は純粋です。時刻も乱数も I/O も使いません。
//!
//! 同じ入力なら必ず同じ点になります。だから閾値シミュレーション（仕様書3章）が
//! 成立します。ここに「現在時刻で減衰させる」ようなものを1つ入れた瞬間、
//! 過去の要求を再評価しても同じ点にならなくなり、シミュレーションが嘘になります。
//!
//! </div>

use serde::{Deserialize, Serialize};

use crate::action::ActionRequest;
use crate::detect::{KindCount, PiiKind, Scan};
use crate::policy::{Decision, Policy, Thresholds};

/// スコアに効いた要素。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Component {
    pub factor: Factor,
    /// 何に当たったか。**個人情報は入れない**（"send_external" などの区分名だけ）。
    pub matched: String,
    /// その要素の素点 0〜100。
    pub raw: u8,
    /// 重み。
    pub weight: u8,
    /// 合計への寄与。
    pub points: f32,
    /// なぜその点なのか。画面にそのまま出す。
    pub why: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Factor {
    Action,
    Destination,
    DataClass,
    Pii(PiiKind),
}

impl Factor {
    pub fn label(self) -> String {
        match self {
            Self::Action => "操作の種類".to_string(),
            Self::Destination => "送信先".to_string(),
            Self::DataClass => "データ区分".to_string(),
            Self::Pii(k) => format!("PII: {}", k.label()),
        }
    }
}

/// 判定の結果。
///
/// <div class="warning">
///
/// 【重要】`thresholds` と `policy_version` をここに同梱しています。
///
/// 仕様書7章「そのときの閾値を必ず残してください」。監査ログを書く側の
/// 気づかいに任せず、**判定結果そのものが閾値を連れて歩く**形にしました。
/// 保存するときに書き忘れる余地がありません。
///
/// </div>
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub score: u8,
    pub decision: Decision,
    pub thresholds: Thresholds,
    pub policy_version: String,
    pub components: Vec<Component>,
    /// 100 で切る前の合計。**判定には使いません。**
    ///
    /// <div class="warning">
    ///
    /// 【重要】諏訪の指示（第4段階）:
    ///
    /// > 「100点だが危険要因は1つ」と「100点で危険要因が3つ」は、監査上まったく違う。
    ///
    /// 判定はどちらも HIGH で同じですが、記録としては別物です。
    /// 頭打ちで潰れた分をここに残しておくと、内訳表示で違いを見せられます。
    ///
    /// </div>
    pub raw_total: u16,
    /// 合計が 100 を超えて頭打ちになったか。
    ///
    /// 【重要】黙って丸めると「なぜ82点か」の説明が合わなくなります。
    pub clamped: bool,
    /// 走査を終えているか。false なら「PIIは無かった」と言えない。
    pub conclusive: bool,
    /// 走査していないために引き上げたか。
    pub raised_by_uncertainty: bool,
    /// 種別と件数のみ。**平文は入っていない**（仕様書5章・7章）。
    pub detected: Vec<KindCount>,
}

impl RiskAssessment {
    /// 人手に回るか。
    pub fn needs_human(&self) -> bool {
        self.decision.needs_human()
    }
}

/// 判定する。純粋関数。
pub fn assess(request: &ActionRequest, scan: &Scan, policy: &Policy) -> RiskAssessment {
    let mut components = Vec::new();

    // ── 操作の種類 ──
    let raw = policy.points_for_action(request.action);
    components.push(Component {
        factor: Factor::Action,
        matched: format!("{:?}", request.action),
        raw,
        weight: policy.weights.action,
        points: contribution(raw, policy.weights.action, 100),
        why: format!("{}は素点{raw}", request.action.label()),
    });

    // ── 送信先 ──
    let raw = policy.points_for_destination(request.destination);
    components.push(Component {
        factor: Factor::Destination,
        matched: format!("{:?}", request.destination),
        raw,
        weight: policy.weights.destination,
        points: contribution(raw, policy.weights.destination, 100),
        why: format!("{}への送信は素点{raw}", request.destination.label()),
    });

    // ── データ区分 ──
    let raw = policy.points_for_data_class(request.data_class);
    let why = if matches!(request.data_class, crate::action::DataClass::Undeclared) {
        format!("データ区分の申告がありません（素点{raw}）。公開扱いにはしません")
    } else {
        format!("{}は素点{raw}", request.data_class.label())
    };
    components.push(Component {
        factor: Factor::DataClass,
        matched: format!("{:?}", request.data_class),
        raw,
        weight: policy.weights.data_class,
        points: contribution(raw, policy.weights.data_class, 100),
        why,
    });

    // ── 検出された PII ──
    //
    // 種別ごとに、いちばん高い確信度のものを代表にする。
    // 件数は「同じ種別が何件あったか」として why に出す。
    for count in scan.summary() {
        let base = policy.points_for_pii(count.kind);
        let confidence = highest_confidence(scan, count.kind);
        let scale = policy.scale(confidence);
        // 確信度で割り引く。Low の氏名を満点で数えない。
        let raw = ((u32::from(base) * scale) / 100).min(100) as u8;
        components.push(Component {
            factor: Factor::Pii(count.kind),
            matched: format!("{:?}", count.kind),
            raw,
            weight: policy.weights.pii,
            points: contribution(raw, policy.weights.pii, 100),
            why: format!(
                "{}を{}件検出（確信度 {:?}。素点{base}の{scale}%）",
                count.kind.label(),
                count.count,
                confidence
            ),
        });
    }

    let total: f32 = components.iter().map(|c| c.points).sum();
    let clamped = total > 100.0;
    // 【重要】切る前の合計を残す。判定には使わない（諏訪の指示・第4段階）。
    let raw_total = total.round().max(0.0) as u16;
    let score = total.round().clamp(0.0, 100.0) as u8;

    // 走査していない本文は、下限まで引き上げる。
    //
    // 【重要】ここがこの関数でいちばん大事な3行です。
    // 「PIIが見つからなかった」と「見ていない」を型で分けた意味は、ここで効きます。
    // 分けていなければ、読めなかった本文が0件として LOW で自動承認されます。
    let by_score = policy.decide(score);
    let conclusive = scan.is_conclusive();
    let decision = if conclusive {
        by_score
    } else {
        by_score.max(policy.unscanned_floor)
    };

    RiskAssessment {
        score,
        decision,
        thresholds: policy.thresholds,
        policy_version: policy.version.clone(),
        components,
        raw_total,
        clamped,
        conclusive,
        raised_by_uncertainty: decision != by_score,
        detected: scan.summary(),
    }
}

/// 素点 × 重み ÷ 100。
fn contribution(raw: u8, weight: u8, divisor: u32) -> f32 {
    (f32::from(raw) * f32::from(weight)) / divisor as f32
}

fn highest_confidence(scan: &Scan, kind: PiiKind) -> crate::detect::Confidence {
    scan.findings()
        .iter()
        .filter(|f| f.kind == kind)
        .map(|f| f.confidence)
        .max()
        .unwrap_or(crate::detect::Confidence::Low)
}

/// 保存済みの内訳から、閾値だけを変えて判定し直す（仕様書3章）。
///
/// <div class="warning">
///
/// 【重要】ここが閾値シミュレーションの中身です。
///
/// 内訳（要素・素点・重み）を保存してあるので、**本文を読み直さずに**
/// 再判定できます。つまり**平文を消したあとでもシミュレーションできます**。
/// 個人情報を持ち続けなくてよい、という設計上の利点がここに出ます。
///
/// </div>
pub fn redecide(assessment: &RiskAssessment, thresholds: Thresholds, floor: Decision) -> Decision {
    let by_score = if assessment.score < thresholds.medium_at {
        Decision::Low
    } else if assessment.score < thresholds.high_at {
        Decision::Medium
    } else {
        Decision::High
    };
    if assessment.conclusive {
        by_score
    } else {
        by_score.max(floor)
    }
}

/// 重みを変えた場合の点を、内訳から計算し直す（仕様書3章）。
///
/// 素点を保存してあるので、こちらも本文なしで再計算できます。
pub fn rescore(assessment: &RiskAssessment, weights: &crate::policy::Weights) -> u8 {
    let total: f32 = assessment
        .components
        .iter()
        .map(|c| {
            let w = match c.factor {
                Factor::Action => weights.action,
                Factor::Destination => weights.destination,
                Factor::DataClass => weights.data_class,
                Factor::Pii(_) => weights.pii,
            };
            contribution(c.raw, w, 100)
        })
        .sum();
    total.round().clamp(0.0, 100.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{ActionKind, DataClass, Destination, Payload};
    use crate::detect::{DetectConfig, SkipReason, scan as run_scan};

    fn request(
        action: ActionKind,
        dest: Destination,
        class: DataClass,
        text: &str,
    ) -> ActionRequest {
        ActionRequest {
            agent_id: "agent-001".to_string(),
            action,
            destination: dest,
            data_class: class,
            payload: Payload {
                text: text.to_string(),
            },
        }
    }

    fn assess_text(
        action: ActionKind,
        dest: Destination,
        class: DataClass,
        text: &str,
    ) -> RiskAssessment {
        let req = request(action, dest, class, text);
        let scan = run_scan(text, &DetectConfig::default());
        assess(&req, &scan, &Policy::provisional())
    }

    #[test]
    fn 内訳の合計が点数になること() {
        // 「なぜ82点か」が説明できること（仕様書4章）。
        let a = assess_text(
            ActionKind::Read,
            Destination::Internal,
            DataClass::Public,
            "在庫を確認します",
        );
        let sum: f32 = a.components.iter().map(|c| c.points).sum();
        assert_eq!(a.score, sum.round() as u8);
        assert!(!a.clamped);
    }

    #[test]
    fn 頭打ちで潰れた分が残ること() {
        // 【重要】判定はどちらも HIGH でも、記録としては別物。
        //   「100点だが危険要因は1つ」と「100点で危険要因が3つ」は、監査上まったく違う。
        // どちらも頭打ちで100点。違うのは、潰れた量。
        let fewer = assess_text(
            ActionKind::Delete,
            Destination::ExternalAi,
            DataClass::Sensitive,
            "カード 4242-4242-4242-4242 で決済した記録を削除します。連絡先 090-1234-5678",
        );
        let more = assess_text(
            ActionKind::Delete,
            Destination::ExternalAi,
            DataClass::Sensitive,
            "山田太郎さん（1980年3月15日生・090-1234-5678）のカード 4242-4242-4242-4242 と \
             メール taro@example.com の記録を削除します",
        );

        assert_eq!(fewer.score, 100);
        assert_eq!(more.score, 100, "表示上の点は同じ");
        assert!(
            more.raw_total > fewer.raw_total,
            "潰れた分が残っていないので、監査上この2件を区別できない（{} vs {}）",
            fewer.raw_total,
            more.raw_total
        );
    }

    #[test]
    fn 生の合計は判定に使わないこと() {
        // raw_total は記録用。三分岐は 0〜100 に切ったあとの score で決める。
        let a = assess_text(
            ActionKind::Delete,
            Destination::ExternalAi,
            DataClass::Sensitive,
            "山田太郎さん（1980年3月15日生・090-1234-5678）カード 4242-4242-4242-4242",
        );
        assert!(a.raw_total > 100);
        assert_eq!(a.score, 100);
        assert_eq!(a.decision, Policy::provisional().decide(a.score));
    }

    #[test]
    fn 頭打ちになったことを隠さないこと() {
        // 合計が100を超えても表示は100だが、丸めた事実は残す。
        let a = assess_text(
            ActionKind::Delete,
            Destination::ExternalAi,
            DataClass::Sensitive,
            "山田太郎さん（1980年3月15日生・090-1234-5678）カード 4242-4242-4242-4242",
        );
        assert_eq!(a.score, 100);
        assert!(a.clamped, "頭打ちの事実が残っていない");
    }

    #[test]
    fn 安全な要求は低く出ること() {
        let a = assess_text(
            ActionKind::Read,
            Destination::Internal,
            DataClass::Public,
            "在庫を確認します",
        );
        assert_eq!(a.decision, Decision::Low);
        assert!(!a.needs_human());
    }

    #[test]
    fn 外部aiへの個人情報は高く出ること() {
        let a = assess_text(
            ActionKind::SendExternal,
            Destination::ExternalAi,
            DataClass::PersonalInformation,
            "山田太郎さん（1980年3月15日生・090-1234-5678）の検査結果について",
        );
        assert_eq!(a.decision, Decision::High);
        assert!(a.needs_human());
    }

    #[test]
    fn 走査していない本文を自動承認しないこと() {
        // 【重要】この試験がこの段階でいちばん大事です。
        // 「見ていない」を「0件」として扱うと、いちばん危ないものが素通りします。
        let req = request(
            ActionKind::Read,
            Destination::Internal,
            DataClass::Public,
            "（読めなかった本文）",
        );
        let unscanned = Scan::NotScanned {
            reason: SkipReason::UnsupportedEncoding,
        };
        let a = assess(&req, &unscanned, &Policy::provisional());

        assert!(a.score < a.thresholds.medium_at, "点数そのものは低いはず");
        assert_ne!(a.decision, Decision::Low, "点が低いからと自動承認している");
        assert!(a.raised_by_uncertainty, "引き上げた事実が残っていない");
        assert!(!a.conclusive);
    }

    #[test]
    fn 判定結果が閾値を連れて歩くこと() {
        // 監査ログに「そのときの閾値」を書き忘れられない形（仕様書7章）。
        let a = assess_text(
            ActionKind::Read,
            Destination::Internal,
            DataClass::Public,
            "在庫を確認します",
        );
        assert_eq!(a.thresholds, Policy::provisional().thresholds);
        assert_eq!(a.policy_version, "provisional-0");
    }

    #[test]
    fn 応答に平文が混ざらないこと() {
        // 仕様書7章「監査ログに平文のPIIを残さない」。
        let text = "山田太郎さん（1980年3月15日生・090-1234-5678）の検査結果";
        let a = assess_text(
            ActionKind::SendExternal,
            Destination::ExternalAi,
            DataClass::PersonalInformation,
            text,
        );
        let json = serde_json::to_string(&a).unwrap();
        for leak in ["山田", "太郎", "1980", "5678"] {
            assert!(!json.contains(leak), "判定結果に平文が混ざっている: {leak}");
        }
    }

    #[test]
    fn 閾値を変えるだけの再判定は本文を要らないこと() {
        // 【重要】これが閾値シミュレーションの土台。
        // 平文を消したあとでも、保存済みの内訳だけで再判定できる。
        let a = assess_text(
            ActionKind::Write,
            Destination::External,
            DataClass::Internal,
            "取引先へ発注書を送ります",
        );
        let strict = Thresholds::new(10, 20).unwrap();
        let loose = Thresholds::new(90, 95).unwrap();

        assert_eq!(redecide(&a, strict, Decision::Medium), Decision::High);
        assert_eq!(redecide(&a, loose, Decision::Medium), Decision::Low);
    }

    #[test]
    fn 走査していない要求はシミュレーションでも自動承認にしないこと() {
        let req = request(
            ActionKind::Read,
            Destination::Internal,
            DataClass::Public,
            "x",
        );
        let a = assess(
            &req,
            &Scan::NotScanned {
                reason: SkipReason::UnsupportedEncoding,
            },
            &Policy::provisional(),
        );
        // どれだけ緩めても LOW にしてはいけない。
        let very_loose = Thresholds::new(99, 100).unwrap();
        assert_ne!(redecide(&a, very_loose, Decision::Medium), Decision::Low);
    }

    #[test]
    fn 重みを変えた再計算も本文を要らないこと() {
        let a = assess_text(
            ActionKind::SendExternal,
            Destination::ExternalAi,
            DataClass::PersonalInformation,
            "山田太郎さんの検査結果",
        );
        let pii_heavy = crate::policy::Weights {
            action: 10,
            destination: 10,
            data_class: 10,
            pii: 60,
        };
        let pii_light = crate::policy::Weights {
            action: 10,
            destination: 10,
            data_class: 10,
            pii: 5,
        };
        assert!(rescore(&a, &pii_heavy) > rescore(&a, &pii_light));
    }

    #[test]
    fn 同じ入力なら何度でも同じ点になること() {
        // 純粋関数であること。ここが崩れるとシミュレーションが嘘になる。
        let text = "山田太郎さん（1980年3月15日生）の件";
        let first = assess_text(
            ActionKind::SendExternal,
            Destination::ExternalAi,
            DataClass::PersonalInformation,
            text,
        );
        for _ in 0..5 {
            let again = assess_text(
                ActionKind::SendExternal,
                Destination::ExternalAi,
                DataClass::PersonalInformation,
                text,
            );
            assert_eq!(first.score, again.score);
            assert_eq!(first.components, again.components);
        }
    }
}
