//! 配点・重み・閾値。
//!
//! <div class="warning">
//!
//! 【重要】このファイルに「採用された値」はまだ入っていません。
//!
//! 仕様書4章「閾値と重みは諏訪が決める。AIが独自に決定しないこと」。
//! 第4段階で案を複数出し、サンプル要求がどう三分岐するかを実測して示し、
//! 諏訪が選定します。
//!
//! ここにあるのは [`Policy::provisional`]（暫定値）だけです。
//! **動かすために必要だから置いてあるだけで、根拠はありません。**
//! 名前と doc に「暫定」と書いてあるのは、うっかり採用値として扱わないためです。
//!
//! </div>

use serde::{Deserialize, Serialize};

use crate::action::{ActionKind, DataClass, Destination};
use crate::detect::{Confidence, PiiKind};

/// 三分岐の境目。
///
/// `score < medium_at` → LOW、`< high_at` → MEDIUM、それ以上 → HIGH。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thresholds {
    pub medium_at: u8,
    pub high_at: u8,
}

/// 閾値が壊れている。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThresholdError {
    /// LOW の上限が HIGH の下限を超えている。三分岐が成立しない。
    Inverted { medium_at: u8, high_at: u8 },
}

impl Thresholds {
    /// 【重要】順序を検証してから作る。
    ///
    /// `medium_at > high_at` を許すと、MEDIUM がどこにも存在しなくなります。
    /// 設定ファイルを手で書き換える運用なので、ここで弾かないと
    /// 「なぜか承認待ちが出ない」という形で表面化します。
    pub fn new(medium_at: u8, high_at: u8) -> Result<Self, ThresholdError> {
        if medium_at > high_at {
            return Err(ThresholdError::Inverted { medium_at, high_at });
        }
        Ok(Self { medium_at, high_at })
    }
}

/// 要素ごとの配点。0〜100 の素点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Points {
    pub action_read: u8,
    pub action_write: u8,
    pub action_delete: u8,
    pub action_send_external: u8,

    pub destination_internal: u8,
    pub destination_external: u8,
    pub destination_external_ai: u8,

    pub data_public: u8,
    pub data_internal: u8,
    pub data_personal: u8,
    pub data_sensitive: u8,
    /// 申告が無い場合。**public と同じにしないこと。**
    pub data_undeclared: u8,

    pub pii_email: u8,
    pub pii_phone: u8,
    pub pii_birth_date: u8,
    pub pii_postal_code: u8,
    pub pii_credit_card: u8,
    pub pii_my_number: u8,
    pub pii_person_name: u8,
}

/// 重み。要素ごとに、素点をどれだけ効かせるか。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Weights {
    pub action: u8,
    pub destination: u8,
    pub data_class: u8,
    pub pii: u8,
}

/// 確信度による割引。
///
/// 【重要】確信度をスコアにどう効かせるかは、仕様書に書かれていません。
/// ここは第4段階で決める対象です（第1段階の修正提案②）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfidenceScale {
    /// 百分率。`50` なら素点を半分にする。
    pub high: u8,
    pub medium: u8,
    pub low: u8,
}

impl ConfidenceScale {
    fn of(&self, c: Confidence) -> u32 {
        u32::from(match c {
            Confidence::High => self.high,
            Confidence::Medium => self.medium,
            Confidence::Low => self.low,
        })
    }
}

/// 判定に使う設定ひとそろい。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    /// 監査ログに残す版。「そのときの閾値」を後から引くのに使う（仕様書7章）。
    pub version: String,
    pub label: String,
    pub thresholds: Thresholds,
    pub weights: Weights,
    pub points: Points,
    pub confidence_scale: ConfidenceScale,
    /// 走査できなかった要求を、最低でもここまで上げる。
    ///
    /// 【重要】走査していない本文を LOW で自動承認しないための下限です。
    /// 「PIIが見つからなかった」と「見ていない」を分けた意味が、ここで効きます。
    pub unscanned_floor: Decision,
    /// 確証のない氏名を含む要求を、最低でもここまで上げる。
    ///
    /// <div class="warning">
    ///
    /// 【重要】諏訪の指示（第4段階）:
    ///
    /// > 辞書外の姓＋様が Medium に落ちるのは構いません。ただし LOW には落とさないでください。
    /// > 「氏名かもしれないが確証がない」は、「氏名がない」とは違います。
    /// > 人に見せる側に倒してください。
    ///
    /// `unscanned_floor` と同じ形です。確信度が High でない氏名は、
    /// 点数がいくら低くても自動承認しません。
    ///
    /// <b>Low（形が似ているだけ）にも同じ扱いを当てています。</b>
    /// Medium より確証が弱いものを自動承認して、Medium だけ止めるのは筋が通らないためです。
    /// Medium だけに限る形が良ければ、そう直します。
    ///
    /// </div>
    pub uncertain_name_floor: Decision,
}

/// 三分岐の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Decision {
    Low,
    Medium,
    High,
}

impl Decision {
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "自動承認",
            Self::Medium => "承認待ち",
            Self::High => "承認待ち（マスキング案つき）",
        }
    }

    /// 人手に回るか。
    pub fn needs_human(self) -> bool {
        !matches!(self, Self::Low)
    }
}

impl Policy {
    /// 暫定値。**採用されたものではありません。**
    ///
    /// 第3段階まで動かすために置いてあるだけで、根拠のある数字ではありません。
    /// 第4段階で案を出し、実測を見て諏訪が選定します。
    pub fn provisional() -> Self {
        Self {
            version: "provisional-0".to_string(),
            label: "暫定（未採用）".to_string(),
            thresholds: Thresholds {
                medium_at: 30,
                high_at: 70,
            },
            weights: Weights {
                action: 25,
                destination: 25,
                data_class: 25,
                pii: 25,
            },
            points: Points {
                action_read: 10,
                action_write: 40,
                action_delete: 80,
                action_send_external: 70,

                destination_internal: 10,
                destination_external: 60,
                destination_external_ai: 90,

                data_public: 0,
                data_internal: 30,
                data_personal: 80,
                data_sensitive: 100,
                data_undeclared: 50,

                pii_email: 40,
                pii_phone: 50,
                pii_birth_date: 60,
                pii_postal_code: 40,
                pii_credit_card: 100,
                pii_my_number: 100,
                pii_person_name: 60,
            },
            confidence_scale: ConfidenceScale {
                high: 100,
                medium: 70,
                low: 40,
            },
            unscanned_floor: Decision::Medium,
            uncertain_name_floor: Decision::Medium,
        }
    }

    /// 設定ファイルの中身から読む。
    ///
    /// 【重要】ファイルを開くのは呼ぶ側の仕事です。ここは文字列を受け取るだけ。
    /// こうしておけば、判定コアが I/O を持たずに済みます（仕様書4章）。
    pub fn from_toml(text: &str) -> Result<Self, PolicyError> {
        let policy: Policy =
            toml::from_str(text).map_err(|e| PolicyError::Malformed(e.to_string()))?;
        policy.validate()?;
        Ok(policy)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("Policy は必ず TOML にできる")
    }

    fn validate(&self) -> Result<(), PolicyError> {
        Thresholds::new(self.thresholds.medium_at, self.thresholds.high_at)
            .map_err(PolicyError::Threshold)?;
        Ok(())
    }

    pub(crate) fn points_for_action(&self, a: ActionKind) -> u8 {
        match a {
            ActionKind::Read => self.points.action_read,
            ActionKind::Write => self.points.action_write,
            ActionKind::Delete => self.points.action_delete,
            ActionKind::SendExternal => self.points.action_send_external,
        }
    }

    pub(crate) fn points_for_destination(&self, d: Destination) -> u8 {
        match d {
            Destination::Internal => self.points.destination_internal,
            Destination::External => self.points.destination_external,
            Destination::ExternalAi => self.points.destination_external_ai,
        }
    }

    pub(crate) fn points_for_data_class(&self, c: DataClass) -> u8 {
        match c {
            DataClass::Public => self.points.data_public,
            DataClass::Internal => self.points.data_internal,
            DataClass::PersonalInformation => self.points.data_personal,
            DataClass::Sensitive => self.points.data_sensitive,
            DataClass::Undeclared => self.points.data_undeclared,
        }
    }

    pub(crate) fn points_for_pii(&self, k: PiiKind) -> u8 {
        match k {
            PiiKind::Email => self.points.pii_email,
            PiiKind::PhoneNumber => self.points.pii_phone,
            PiiKind::BirthDate => self.points.pii_birth_date,
            PiiKind::PostalCode => self.points.pii_postal_code,
            PiiKind::CreditCard => self.points.pii_credit_card,
            PiiKind::MyNumber => self.points.pii_my_number,
            PiiKind::PersonName => self.points.pii_person_name,
        }
    }

    pub(crate) fn scale(&self, c: Confidence) -> u32 {
        self.confidence_scale.of(c)
    }

    pub(crate) fn decide(&self, score: u8) -> Decision {
        if score < self.thresholds.medium_at {
            Decision::Low
        } else if score < self.thresholds.high_at {
            Decision::Medium
        } else {
            Decision::High
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    Malformed(String),
    Threshold(ThresholdError),
}

impl PolicyError {
    pub fn message(&self) -> String {
        match self {
            Self::Malformed(e) => format!("設定を読めません: {e}"),
            Self::Threshold(ThresholdError::Inverted { medium_at, high_at }) => format!(
                "閾値が逆です（MEDIUM {medium_at} > HIGH {high_at}）。この設定では MEDIUM が出ません"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 逆さまの閾値を作れないこと() {
        // 【重要】これを許すと MEDIUM がどこにも無くなる。
        // 「なぜか承認待ちが出ない」という形でしか表面化しない。
        assert_eq!(
            Thresholds::new(80, 30),
            Err(ThresholdError::Inverted {
                medium_at: 80,
                high_at: 30
            })
        );
        assert!(Thresholds::new(30, 70).is_ok());
        assert!(
            Thresholds::new(50, 50).is_ok(),
            "MEDIUM が空でも設定としては成立する"
        );
    }

    #[test]
    fn 三分岐の境目が閾値どおりであること() {
        let p = Policy::provisional(); // 30 / 70
        assert_eq!(p.decide(29), Decision::Low);
        assert_eq!(p.decide(30), Decision::Medium, "境目はその値を含む");
        assert_eq!(p.decide(69), Decision::Medium);
        assert_eq!(p.decide(70), Decision::High);
        assert_eq!(p.decide(100), Decision::High);
    }

    #[test]
    fn 設定を書き出して読み直せること() {
        // 設定ファイルに外出しできること（仕様書4章）。
        let original = Policy::provisional();
        let text = original.to_toml();
        let parsed = Policy::from_toml(&text).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn 壊れた設定は読み込みで弾くこと() {
        let mut broken = Policy::provisional();
        broken.thresholds = Thresholds {
            medium_at: 90,
            high_at: 20,
        };
        let err = Policy::from_toml(&broken.to_toml()).unwrap_err();
        assert!(err.message().contains("閾値が逆"), "{}", err.message());
    }

    #[test]
    fn 申告なしを公開と同じ扱いにしないこと() {
        // 【重要】ここが同点だと、data_class を付け忘れるほど安全に判定される。
        let p = Policy::provisional();
        assert!(
            p.points_for_data_class(DataClass::Undeclared)
                > p.points_for_data_class(DataClass::Public),
            "申告なしが公開と同じかそれ以下になっている"
        );
    }

    #[test]
    fn 暫定であることが名前から分かること() {
        // 採用値と取り違えないための印。第4段階で置き換える。
        let p = Policy::provisional();
        assert!(p.version.contains("provisional"));
        assert!(p.label.contains("暫定"));
    }
}
