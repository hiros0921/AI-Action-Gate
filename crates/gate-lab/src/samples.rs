//! 案を比べるためのサンプル要求。
//!
//! <div class="warning">
//!
//! 【重要】案より先に、材料を確定させます（諏訪の指示・第4段階）。
//!
//! > 材料が偏っていると、どの案を選んでも判断がずれます。
//! > 材料の作り方が結果を歪めるためです。
//!
//! 第3段階で分かったとおり、**素点100の要求は閾値を動かしても分岐が変わりません**。
//! 頭打ちだからです。同じ理由で、極端に低いものも動きません。
//! 案の差が出るのは中間帯だけなので、そこを最も厚くしてあります（型3）。
//!
//! </div>
//!
//! <div class="warning">
//!
//! 【重要】ここに出てくる氏名・電話番号・生年月日・企業名は**すべて架空**です（仕様書9章）。
//! メールは試験用に予約されたドメイン（example.com / RFC 2606）を使っています。
//!
//! </div>

use gate_core::action::{ActionKind, ActionRequest, DataClass, Destination, Payload};
use gate_core::detect::DetectConfig;

/// サンプルの型（諏訪の指定・第4段階）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// 型1: 社内データの read。自動承認されてよい。
    InternalRead,
    /// 型2: 患者情報を外部AIへ。必ず承認待ち。
    PatientToExternalAi,
    /// 型3: 境目。案によって分岐が変わる。**最も厚くする。**
    Borderline,
    /// 型4: 本文が読めなかった。点が低くても必ず承認待ち。
    Unscannable,
    /// 型5: PIIは無いが外部送信。判断が分かれる。
    ExternalNoPii,
    /// 型6: PIIはあるが送信先が社内。判断が分かれる。
    InternalWithPii,
    /// 型7: delete 系。送信先に関わらず慎重に。
    Delete,
}

impl Kind {
    pub fn number(self) -> u8 {
        match self {
            Self::InternalRead => 1,
            Self::PatientToExternalAi => 2,
            Self::Borderline => 3,
            Self::Unscannable => 4,
            Self::ExternalNoPii => 5,
            Self::InternalWithPii => 6,
            Self::Delete => 7,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::InternalRead => "社内の read",
            Self::PatientToExternalAi => "患者情報を外部AIへ",
            Self::Borderline => "境目",
            Self::Unscannable => "読めなかった",
            Self::ExternalNoPii => "PIIなし・外部送信",
            Self::InternalWithPii => "PIIあり・社内",
            Self::Delete => "delete 系",
        }
    }

    /// この型に期待する挙動。案の合否を機械的に判定するのに使う。
    pub fn expectation(self) -> Expectation {
        match self {
            // 自動承認されてよい（回ってもよいが、少ないほうが良い）。
            Self::InternalRead => Expectation::ShouldPassAuto,
            // 【重要】ここが自動承認された案は、他がどれだけ良くても採れない。
            Self::PatientToExternalAi | Self::Unscannable => Expectation::MustHold,
            Self::Borderline | Self::ExternalNoPii | Self::InternalWithPii | Self::Delete => {
                Expectation::Split
            }
        }
    }
}

/// 期待する挙動。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expectation {
    /// 自動承認されてよい。人手に回ると損（ただし違反ではない）。
    ShouldPassAuto,
    /// **必ず**承認待ちになること。ここが破れた案は不採用。
    MustHold,
    /// 案によって分かれてよい。どちらが良いかは運用の判断。
    Split,
}

/// サンプル1件。
pub struct Sample {
    pub id: &'static str,
    pub kind: Kind,
    pub note: &'static str,
    pub request: ActionRequest,
    /// 走査の設定。型4だけ上限を小さくして「読めなかった」を作る。
    pub detect: DetectConfig,
}

fn req(
    agent: &str,
    action: ActionKind,
    destination: Destination,
    data_class: DataClass,
    text: &str,
) -> ActionRequest {
    ActionRequest {
        agent_id: agent.to_string(),
        action,
        destination,
        data_class,
        payload: Payload {
            text: text.to_string(),
        },
    }
}

fn sample(id: &'static str, kind: Kind, note: &'static str, request: ActionRequest) -> Sample {
    Sample {
        id,
        kind,
        note,
        request,
        detect: DetectConfig::default(),
    }
}

/// 走査できない1件を作る。
///
/// 【重要】本文を長くして上限を超えさせています。設定側で細工していません。
/// 実運用でも「大きすぎて走査を諦めた」はこの形で起きます。
fn unscannable(id: &'static str, note: &'static str, request: ActionRequest) -> Sample {
    Sample {
        id,
        kind: Kind::Unscannable,
        note,
        request,
        detect: DetectConfig {
            payload_limit: 256,
            ..Default::default()
        },
    }
}

/// 比較に使うサンプル一式。
pub fn all() -> Vec<Sample> {
    // 【重要】`use ...::*` を使わない。
    // Destination::Internal と DataClass::Internal が同名で、どちらか分からなくなる。
    use ActionKind::{Delete, Read, SendExternal, Write};
    use DataClass::{PersonalInformation, Public, Sensitive, Undeclared};
    use Destination::{External, ExternalAi};

    vec![
        // ── 型1: 社内の read。自動承認されてよい ──────────────
        sample(
            "S1-a",
            Kind::InternalRead,
            "在庫の照会。PIIなし",
            req(
                "agent-001",
                Read,
                Destination::Internal,
                Public,
                "倉庫Aの在庫一覧を確認します。対象は全SKUです。",
            ),
        ),
        sample(
            "S1-b",
            Kind::InternalRead,
            "社内文書の参照",
            req(
                "agent-002",
                Read,
                Destination::Internal,
                DataClass::Internal,
                "社内の運用手順書から、バックアップ手順の節を読み取ります。",
            ),
        ),
        sample(
            "S1-c",
            Kind::InternalRead,
            "ログの参照",
            req(
                "agent-001",
                Read,
                Destination::Internal,
                DataClass::Internal,
                "直近1時間のアクセスログから、エラー率を集計します。",
            ),
        ),
        // ── 型2: 患者情報を外部AIへ。必ず承認待ち ──────────────
        sample(
            "S2-a",
            Kind::PatientToExternalAi,
            "氏名・生年月日・電話（仕様書2章の例）",
            req(
                "agent-003",
                SendExternal,
                ExternalAi,
                PersonalInformation,
                "山田太郎さん（1980年3月15日生・090-1234-5678）の検査結果について要約してください。",
            ),
        ),
        sample(
            "S2-b",
            Kind::PatientToExternalAi,
            "要配慮情報。氏名は敬称つき",
            req(
                "agent-003",
                SendExternal,
                ExternalAi,
                Sensitive,
                "患者：田中花子さんの服薬履歴を、外部の要約サービスへ送ります。",
            ),
        ),
        sample(
            "S2-c",
            Kind::PatientToExternalAi,
            "氏名の確信度が低い（カタカナ）",
            req(
                "agent-003",
                SendExternal,
                ExternalAi,
                Sensitive,
                "スズキ・ハナコ の診療メモを要約してください。次回受診は来月です。",
            ),
        ),
        // ── 型3: 境目。ここを最も厚くする ─────────────────────
        sample(
            "S3-a",
            Kind::Borderline,
            "社外への書き込み。メール1件",
            req(
                "agent-004",
                Write,
                External,
                DataClass::Internal,
                "取引先へ発注書を送信します。連絡先は order@example.com です。",
            ),
        ),
        sample(
            "S3-b",
            Kind::Borderline,
            "外部AIへ読み取り。PIIなし",
            req(
                "agent-004",
                Read,
                ExternalAi,
                DataClass::Internal,
                "社内規程の要約を、外部の要約サービスに依頼します。個人名は含みません。",
            ),
        ),
        sample(
            "S3-c",
            Kind::Borderline,
            "社外への送信。電話番号1件",
            req(
                "agent-005",
                SendExternal,
                External,
                DataClass::Internal,
                "配送業者へ連絡します。問い合わせ先は 03-1234-5678 です。",
            ),
        ),
        sample(
            "S3-d",
            Kind::Borderline,
            "データ区分の申告なし",
            req(
                "agent-005",
                SendExternal,
                External,
                Undeclared,
                "見積書の内容を先方へ共有します。",
            ),
        ),
        sample(
            "S3-e",
            Kind::Borderline,
            "外部AIへ書き込み。氏名の確信度が低い",
            req(
                "agent-004",
                Write,
                ExternalAi,
                Public,
                "議事録を清書します。出席者はタナカ・タロウとヤマダ・ジロウでした。",
            ),
        ),
        sample(
            "S3-f",
            Kind::Borderline,
            "社内の read だが個人情報",
            req(
                "agent-006",
                Read,
                Destination::Internal,
                PersonalInformation,
                "顧客名簿から連絡先を照会します。taro@example.com と 090-1234-5678 が対象です。",
            ),
        ),
        sample(
            "S3-g",
            Kind::Borderline,
            "社外へ、郵便番号のみ",
            req(
                "agent-005",
                SendExternal,
                External,
                Public,
                "配送先の地域を確認します。〒123-4567 の区域です。",
            ),
        ),
        sample(
            "S3-h",
            Kind::Borderline,
            "社外への書き込み。氏名（辞書一致）",
            req(
                "agent-004",
                Write,
                External,
                PersonalInformation,
                "担当変更の連絡です。後任は山田太郎が務めます。",
            ),
        ),
        // ── 型4: 読めなかった。点が低くても必ず承認待ち ──────────
        unscannable(
            "S4-a",
            "上限を超える本文。社内の read なので点は低い",
            req(
                "agent-007",
                Read,
                Destination::Internal,
                Public,
                &"添付された長大な点検記録の本文です。".repeat(40),
            ),
        ),
        unscannable(
            "S4-b",
            "上限を超える本文。外部AIへ",
            req(
                "agent-007",
                SendExternal,
                ExternalAi,
                Undeclared,
                &"取り込んだ帳票のテキストが続きます。".repeat(40),
            ),
        ),
        // ── 型5: PIIは無いが外部送信 ─────────────────────────
        sample(
            "S5-a",
            Kind::ExternalNoPii,
            "公開情報を社外へ",
            req(
                "agent-008",
                SendExternal,
                External,
                Public,
                "公開済みのプレスリリース本文を、社外の翻訳サービスへ送ります。",
            ),
        ),
        sample(
            "S5-b",
            Kind::ExternalNoPii,
            "社内限りの情報を外部AIへ",
            req(
                "agent-008",
                SendExternal,
                ExternalAi,
                DataClass::Internal,
                "来期の要員計画の草案を、外部AIに要約させます。個人名は含みません。",
            ),
        ),
        sample(
            "S5-c",
            Kind::ExternalNoPii,
            "社外への書き込み。公開情報",
            req(
                "agent-008",
                Write,
                External,
                Public,
                "公開中の製品仕様を、取引先のポータルへ登録します。",
            ),
        ),
        // ── 型6: PIIはあるが送信先が社内 ─────────────────────
        sample(
            "S6-a",
            Kind::InternalWithPii,
            "社内で個人情報を書き込む",
            req(
                "agent-009",
                Write,
                Destination::Internal,
                PersonalInformation,
                "山田太郎さんの連絡先（090-1234-5678）を顧客台帳へ登録します。",
            ),
        ),
        sample(
            "S6-b",
            Kind::InternalWithPii,
            "社内でカード番号を扱う",
            req(
                "agent-009",
                Write,
                Destination::Internal,
                Sensitive,
                "決済の照合のため、カード 4242-4242-4242-4242 の下4桁を控えます。",
            ),
        ),
        sample(
            "S6-c",
            Kind::InternalWithPii,
            "社内の read。メールのみ",
            req(
                "agent-009",
                Read,
                Destination::Internal,
                PersonalInformation,
                "問い合わせ元 taro@example.com の過去履歴を参照します。",
            ),
        ),
        // ── 型7: delete 系。送信先に関わらず慎重に ─────────────
        sample(
            "S7-a",
            Kind::Delete,
            "社内の削除。PIIなし",
            req(
                "agent-010",
                Delete,
                Destination::Internal,
                DataClass::Internal,
                "一時ファイルの置き場を削除します。対象は3日以上前のものです。",
            ),
        ),
        sample(
            "S7-b",
            Kind::Delete,
            "社内の削除。個人情報を含む",
            req(
                "agent-010",
                Delete,
                Destination::Internal,
                PersonalInformation,
                "退会した利用者 taro@example.com の登録情報を削除します。",
            ),
        ),
        sample(
            "S7-c",
            Kind::Delete,
            "外部の削除",
            req(
                "agent-010",
                Delete,
                External,
                DataClass::Internal,
                "取引先ポータルに登録した古い仕様書を削除します。",
            ),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use gate_core::detect;
    use gate_core::policy::{Decision, Policy};
    use gate_core::score::assess;

    #[test]
    fn 型ごとに複数件あること() {
        // 諏訪の指示: 「以下の型を、それぞれ複数件用意すること」
        let samples = all();
        for kind in [
            Kind::InternalRead,
            Kind::PatientToExternalAi,
            Kind::Borderline,
            Kind::Unscannable,
            Kind::ExternalNoPii,
            Kind::InternalWithPii,
            Kind::Delete,
        ] {
            let n = samples.iter().filter(|s| s.kind == kind).count();
            assert!(
                n >= 2,
                "型{}（{}）が {n} 件しかない",
                kind.number(),
                kind.label()
            );
        }
    }

    #[test]
    fn 境目がいちばん厚いこと() {
        // 【重要】案の差が出るのは中間帯だけ。ここが薄いと比較にならない。
        let samples = all();
        let borderline = samples
            .iter()
            .filter(|s| s.kind == Kind::Borderline)
            .count();
        for kind in [
            Kind::InternalRead,
            Kind::PatientToExternalAi,
            Kind::Unscannable,
            Kind::ExternalNoPii,
            Kind::InternalWithPii,
            Kind::Delete,
        ] {
            let n = samples.iter().filter(|s| s.kind == kind).count();
            assert!(
                borderline > n,
                "境目({borderline}件)が型{}({n}件)より薄い",
                kind.number()
            );
        }
    }

    #[test]
    fn 境目のサンプルが中間帯に散らばっていること() {
        // 頭打ち（100）と底（0付近）に寄っていたら、閾値を動かしても分岐が変わらない。
        let policy = Policy::provisional();
        let scores: Vec<u8> = all()
            .iter()
            .filter(|s| s.kind == Kind::Borderline)
            .map(|s| {
                let scan = detect::scan(&s.request.payload.text, &s.detect);
                assess(&s.request, &scan, &policy).score
            })
            .collect();

        assert!(
            scores.iter().all(|&s| (20..=80).contains(&s)),
            "境目のはずが中間帯から外れている: {scores:?}"
        );
        let min = *scores.iter().min().unwrap();
        let max = *scores.iter().max().unwrap();
        assert!(max - min >= 20, "境目が1点に固まっている（{min}〜{max}）");
    }

    #[test]
    fn 型4は走査できていないこと() {
        // 「読めなかった」を作れていなければ、型4の試験そのものが無意味になる。
        for s in all().iter().filter(|s| s.kind == Kind::Unscannable) {
            let scan = detect::scan(&s.request.payload.text, &s.detect);
            assert!(!scan.is_conclusive(), "{} が走査できてしまっている", s.id);
        }
    }

    #[test]
    fn 型2と型4は暫定の設定でも承認待ちになること() {
        // 採用値を決める前の確認。ここが破れていたら、サンプルか実装のどちらかがおかしい。
        let policy = Policy::provisional();
        for s in all()
            .iter()
            .filter(|s| s.kind.expectation() == Expectation::MustHold)
        {
            let scan = detect::scan(&s.request.payload.text, &s.detect);
            let a = assess(&s.request, &scan, &policy);
            assert_ne!(
                a.decision,
                Decision::Low,
                "{} が自動承認された（{}点）",
                s.id,
                a.score
            );
        }
    }

    #[test]
    fn 実在の個人情報を含まないこと() {
        // 仕様書9章。メールは RFC 2606 の予約ドメインだけ。
        for s in all() {
            let text = &s.request.payload.text;
            if let Some(at) = text.find('@') {
                assert!(
                    text[at..].starts_with("@example."),
                    "{} が予約ドメイン以外のメールを含む",
                    s.id
                );
            }
        }
    }
}
