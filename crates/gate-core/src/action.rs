//! エージェントから届く実行要求。
//!
//! ここは「何が来たか」を型にするだけ。危険かどうかの判断は [`crate::score`] が持つ。

use serde::{Deserialize, Serialize};

/// 操作の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Read,
    Write,
    Delete,
    SendExternal,
}

impl ActionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Read => "読み取り",
            Self::Write => "書き込み",
            Self::Delete => "削除",
            Self::SendExternal => "外部送信",
        }
    }
}

/// 送信先。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Destination {
    Internal,
    External,
    ExternalAi,
}

impl Destination {
    pub fn label(self) -> &'static str {
        match self {
            Self::Internal => "社内",
            Self::External => "社外",
            Self::ExternalAi => "外部AI",
        }
    }
}

/// データ区分。
///
/// <div class="warning">
///
/// 【重要】`Undeclared`（申告なし）を用意してあるのは、
/// **「public だと言われた」と「何も言われていない」を混同しないため**です。
///
/// エージェントが `data_class` を付け忘れた要求を `Public` として扱うと、
/// 付け忘れるほど安全側に判定される、という逆の挙動になります。
/// 申告が無いことは、それ自体が判断材料です。
///
/// </div>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataClass {
    Public,
    Internal,
    PersonalInformation,
    Sensitive,
    /// 要求に `data_class` が無かった。既定値ではなく「申告が無い」という事実。
    #[serde(other)]
    Undeclared,
}

impl DataClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::Public => "公開",
            Self::Internal => "社内限り",
            Self::PersonalInformation => "個人情報",
            Self::Sensitive => "要配慮",
            Self::Undeclared => "申告なし",
        }
    }
}

/// 実行要求の本体。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRequest {
    pub agent_id: String,
    pub action: ActionKind,
    pub destination: Destination,
    /// 省略された場合は [`DataClass::Undeclared`]。既定を `Public` にしない。
    #[serde(default = "undeclared")]
    pub data_class: DataClass,
    pub payload: Payload,
}

fn undeclared() -> DataClass {
    DataClass::Undeclared
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payload {
    pub text: String,
}

/// 形式の検証（仕様書2章の①）。
///
/// 型で表せない条件だけをここで見る。`action` が知らない値だった、のような
/// 「型にできる誤り」は serde の deserialize で落ちるので、ここには来ない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invalid {
    /// エージェントIDが空。監査ログの「誰が」が埋まらなくなる。
    EmptyAgentId,
    /// 本文が空。検査する対象が無い。
    EmptyPayload,
    /// 本文が長すぎる。走査できる上限を超えている。
    PayloadTooLarge { bytes: usize, limit: usize },
}

impl Invalid {
    pub fn message(&self) -> String {
        match self {
            Self::EmptyAgentId => "agent_id が空です".to_string(),
            Self::EmptyPayload => "payload.text が空です".to_string(),
            Self::PayloadTooLarge { bytes, limit } => {
                format!("payload.text が大きすぎます（{bytes} バイト。上限 {limit}）")
            }
        }
    }
}

impl ActionRequest {
    /// 受け付けてよい形かどうか。
    ///
    /// 見つかった問題を全部返す。1つ目で止めると、直しては送り直しを繰り返させることになる。
    pub fn validate(&self, payload_limit: usize) -> Result<(), Vec<Invalid>> {
        let mut errors = Vec::new();
        if self.agent_id.trim().is_empty() {
            errors.push(Invalid::EmptyAgentId);
        }
        if self.payload.text.trim().is_empty() {
            errors.push(Invalid::EmptyPayload);
        }
        let bytes = self.payload.text.len();
        if bytes > payload_limit {
            errors.push(Invalid::PayloadTooLarge {
                bytes,
                limit: payload_limit,
            });
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(agent: &str, text: &str) -> ActionRequest {
        ActionRequest {
            agent_id: agent.to_string(),
            action: ActionKind::SendExternal,
            destination: Destination::ExternalAi,
            data_class: DataClass::PersonalInformation,
            payload: Payload {
                text: text.to_string(),
            },
        }
    }

    #[test]
    fn 形式が整っていれば通ること() {
        assert!(req("agent-001", "こんにちは").validate(1000).is_ok());
    }

    #[test]
    fn 問題は全部まとめて返すこと() {
        // 1つ目で止めると、直しては送り直しを繰り返させることになる。
        let errors = req("  ", "   ").validate(1000).unwrap_err();
        assert!(errors.contains(&Invalid::EmptyAgentId));
        assert!(errors.contains(&Invalid::EmptyPayload));
    }

    #[test]
    fn 大きすぎる本文は受け付けないこと() {
        let big = "あ".repeat(400); // 1200 バイト
        let errors = req("agent-001", &big).validate(1000).unwrap_err();
        assert!(matches!(errors[0], Invalid::PayloadTooLarge { .. }));
    }

    #[test]
    fn データ区分が無いときは申告なしとして扱うこと() {
        // 【重要】ここが Public に落ちると、付け忘れるほど安全に判定される。
        let json = r#"{
            "agent_id": "agent-001",
            "action": "send_external",
            "destination": "external_ai",
            "payload": { "text": "本文" }
        }"#;
        let parsed: ActionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.data_class, DataClass::Undeclared);
    }

    #[test]
    fn 仕様書の例がそのまま読めること() {
        // 仕様書2章の要求例。フィールド名が食い違っていればここで落ちる。
        let json = r#"{
            "agent_id": "agent-001",
            "action": "send_external",
            "destination": "external_ai",
            "payload": {
                "text": "山田太郎さん（1980年3月15日生・090-1234-5678）の検査結果について..."
            }
        }"#;
        let parsed: ActionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.action, ActionKind::SendExternal);
        assert_eq!(parsed.destination, Destination::ExternalAi);
    }
}
