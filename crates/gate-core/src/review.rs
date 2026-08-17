//! 人が下す判断と、その記録。
//!
//! <div class="warning">
//!
//! 【重要】ここは `approver_id` を**ただの文字列として受け取ります**。
//!
//! それがどこから来たのか（HTTPヘッダ・Cognito・JWT・社内SSO）を、この層は知りません。
//! 諏訪の指示（第2段階の回答）:
//!
//! > ドメイン層は approver_id という文字列を受け取るだけにすること。
//! > どこから来たかを知らない形にする。
//! > 認証を作らないことと、認証を後から入れられないことは、別です。
//!
//! いまはヘッダ `X-Approver-Id` から取っていますが、差し替えるのは
//! 入口（`gate-api` の抽出器）だけで、この層は1行も変わりません。
//!
//! </div>

use serde::{Deserialize, Serialize};

use crate::detect::KindCount;
use crate::policy::{Decision, Thresholds};
use crate::score::RiskAssessment;

/// 人が下した判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// そのまま実行してよい。
    Approved,
    /// 伏せ字にしたうえで実行してよい。
    ApprovedWithMasking,
    /// 実行させない。
    Rejected,
    /// 誰も判断しないまま期限が切れた。
    ///
    /// <div class="warning">
    ///
    /// 【重要】人の「拒否」と分けます（諏訪の指示・第6段階）。
    ///
    /// > 「承認完了後24時間」だと、承認待ちのまま放置された平文が永久に残ります。
    /// > 一定期間で「期限切れ」として閉じて、そこからTTLを開始する形にしてください。
    ///
    /// これを `Rejected` で表すと、監査ログ上「人が拒否した」ことになります。
    /// 実際には誰も見ていません。**判断しなかったことも記録**です。
    ///
    /// </div>
    Expired,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Self::Approved => "承認",
            Self::ApprovedWithMasking => "マスキングして承認",
            Self::Rejected => "拒否",
            Self::Expired => "期限切れ（誰も判断しなかった）",
        }
    }

    /// エージェントに実行を許すか。
    pub fn allows_execution(self) -> bool {
        matches!(self, Self::Approved | Self::ApprovedWithMasking)
    }
}

/// 誰が判断したか。
///
/// 【重要】自動承認（LOW）も「誰が」を持ちます。人ではないので `System` です。
/// 空文字や `"unknown"` で埋めると、あとから「人が承認したのか、自動だったのか」が
/// 区別できなくなります。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reviewer {
    /// 人。`id` の出どころはこの層では問わない。
    Human { approver_id: String },
    /// 自動承認。閾値が下したもの。
    System,
}

impl Reviewer {
    /// 表示用。監査ログにもこの文字列を残す。
    pub fn describe(&self) -> String {
        match self {
            Self::Human { approver_id } => approver_id.clone(),
            Self::System => "(自動承認)".to_string(),
        }
    }

    pub fn is_human(&self) -> bool {
        matches!(self, Self::Human { .. })
    }
}

/// 承認者IDとして受け付けられない値。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApproverError {
    /// 空。誰が承認したか分からない記録を作らない。
    Empty,
    /// 長すぎる。ヘッダに何かを詰め込まれている。
    TooLong { len: usize },
}

impl ApproverError {
    pub fn message(&self) -> String {
        match self {
            Self::Empty => "承認者IDがありません".to_string(),
            Self::TooLong { len } => format!("承認者IDが長すぎます（{len} 文字）"),
        }
    }
}

/// 人の判断を作る。
///
/// 【重要】`approver_id` が空なら作れません。
/// 「誰が承認したか」を空欄のまま監査ログに残す道を、型のレベルで塞いであります。
pub fn human(approver_id: &str) -> Result<Reviewer, ApproverError> {
    let trimmed = approver_id.trim();
    if trimmed.is_empty() {
        return Err(ApproverError::Empty);
    }
    if trimmed.chars().count() > 64 {
        return Err(ApproverError::TooLong {
            len: trimmed.chars().count(),
        });
    }
    Ok(Reviewer::Human {
        approver_id: trimmed.to_string(),
    })
}

/// 監査ログの1行。**追記のみ**（仕様書7章）。
///
/// <div class="warning">
///
/// 【重要】ここに平文の個人情報は入りません。
///
/// 入っているのは検出の**種別と件数**だけです（[`KindCount`]）。
/// 本文そのものを残すと、消せないログに個人情報が永久に残ることになります。
/// 監査ログを消せなくすることと、個人情報を消せなくすることは、別です。
///
/// </div>
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub request_id: String,
    /// いつ。**この層では時刻を取らない**ので、呼ぶ側が渡す。
    pub at: String,
    pub reviewer: Reviewer,
    pub verdict: Verdict,
    /// 自動判定の結果。人がどう覆したかを読むのに要る。
    pub decision: Decision,
    pub score: u8,
    /// 【重要】そのときの閾値（仕様書7章）。
    pub thresholds: Thresholds,
    pub policy_version: String,
    /// 種別と件数のみ。
    pub detected: Vec<KindCount>,
    /// 走査を終えていたか。
    pub conclusive: bool,
}

impl AuditEntry {
    /// 判定結果と人の判断から、監査ログの1行を作る。
    ///
    /// 時刻は引数で受け取る。ここで `SystemTime::now()` を呼ぶと、
    /// この関数の試験に時計が入り込みます。
    pub fn new(
        request_id: &str,
        at: &str,
        reviewer: Reviewer,
        verdict: Verdict,
        assessment: &RiskAssessment,
    ) -> Self {
        Self {
            request_id: request_id.to_string(),
            at: at.to_string(),
            reviewer,
            verdict,
            decision: assessment.decision,
            score: assessment.score,
            // 【重要】判定結果が閾値を連れてくるので、ここで拾い忘れることが無い。
            thresholds: assessment.thresholds,
            policy_version: assessment.policy_version.clone(),
            detected: assessment.detected.clone(),
            conclusive: assessment.conclusive,
        }
    }

    /// 人が自動判定を覆したか。
    ///
    /// HIGH と判定されたものを人が承認した、のような記録を拾うのに使う。
    pub fn overrode_machine(&self) -> bool {
        match self.verdict {
            Verdict::Approved => self.decision == Decision::High,
            Verdict::Rejected => self.decision == Decision::Low,
            // 期限切れは人の判断ではないので、覆したことにならない。
            Verdict::ApprovedWithMasking | Verdict::Expired => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{ActionKind, ActionRequest, DataClass, Destination, Payload};
    use crate::detect::{DetectConfig, scan};
    use crate::policy::Policy;
    use crate::score::assess;

    fn assessment() -> RiskAssessment {
        let text = "山田太郎さん（1980年3月15日生・090-1234-5678）の検査結果";
        let req = ActionRequest {
            agent_id: "agent-001".to_string(),
            action: ActionKind::SendExternal,
            destination: Destination::ExternalAi,
            data_class: DataClass::PersonalInformation,
            payload: Payload {
                text: text.to_string(),
            },
        };
        assess(
            &req,
            &scan(text, &DetectConfig::default()),
            &Policy::provisional(),
        )
    }

    #[test]
    fn 承認者idは文字列として受け取るだけであること() {
        // ドメインは出どころを知らない。ヘッダでも JWT でも同じ形で入る。
        let from_header = human("suwa").unwrap();
        let from_jwt_someday = human("suwa").unwrap();
        assert_eq!(from_header, from_jwt_someday);
    }

    #[test]
    fn 空の承認者idでは記録を作れないこと() {
        // 【重要】「誰が承認したか」が空欄の監査ログを作れないようにする。
        assert_eq!(human(""), Err(ApproverError::Empty));
        assert_eq!(human("   "), Err(ApproverError::Empty));
        assert!(human(&"a".repeat(65)).is_err());
    }

    #[test]
    fn 自動承認と人の承認を区別できること() {
        // 空文字で埋めると、あとからどちらか分からなくなる。
        assert!(!Reviewer::System.is_human());
        assert!(human("suwa").unwrap().is_human());
        assert_eq!(Reviewer::System.describe(), "(自動承認)");
    }

    #[test]
    fn 監査ログがそのときの閾値を持つこと() {
        let a = assessment();
        let entry = AuditEntry::new(
            "req-0001",
            "2026-08-17T12:00:00Z",
            human("suwa").unwrap(),
            Verdict::ApprovedWithMasking,
            &a,
        );
        assert_eq!(entry.thresholds, a.thresholds);
        assert_eq!(entry.policy_version, a.policy_version);
    }

    #[test]
    fn 監査ログに平文が入らないこと() {
        // 仕様書7章。消せないログに個人情報を残さない。
        let entry = AuditEntry::new(
            "req-0001",
            "2026-08-17T12:00:00Z",
            human("suwa").unwrap(),
            Verdict::Approved,
            &assessment(),
        );
        let json = serde_json::to_string(&entry).unwrap();
        for leak in ["山田", "太郎", "1980", "5678", "検査結果"] {
            assert!(!json.contains(leak), "監査ログに平文が混ざっている: {leak}");
        }
        // 種別と件数は残る。
        assert!(json.contains("person_name"));
    }

    #[test]
    fn 期限切れを人の拒否と混同しないこと() {
        // 【重要】どちらも「実行させない」だが、記録としては別物。
        // 拒否は人が見て決めたこと。期限切れは誰も見なかったこと。
        assert!(!Verdict::Expired.allows_execution());
        assert!(!Verdict::Rejected.allows_execution());
        assert_ne!(Verdict::Expired, Verdict::Rejected);

        let entry = AuditEntry::new(
            "req-0001",
            "2026-08-18T12:00:00Z",
            Reviewer::System,
            Verdict::Expired,
            &assessment(),
        );
        assert!(
            !entry.overrode_machine(),
            "誰も判断していないのに覆したことになっている"
        );
        assert!(!entry.reviewer.is_human());
    }

    #[test]
    fn 人が機械を覆したことが分かること() {
        let a = assessment(); // HIGH になる
        assert_eq!(a.decision, Decision::High);

        let approved = AuditEntry::new("r", "t", human("suwa").unwrap(), Verdict::Approved, &a);
        assert!(
            approved.overrode_machine(),
            "HIGH をそのまま承認したことが残らない"
        );

        let masked = AuditEntry::new(
            "r",
            "t",
            human("suwa").unwrap(),
            Verdict::ApprovedWithMasking,
            &a,
        );
        assert!(
            !masked.overrode_machine(),
            "マスキング承認は想定どおりの流れ"
        );
    }
}
