//! DynamoDB への永続化（第6段階）。
//!
//! <div class="warning">
//!
//! <h2>【重要】平文を、判定結果と同じアイテムに置きません</h2>
//!
//! 諏訪の指示（第6段階）:
//!
//! > DynamoDBのTTLはアイテム単位で丸ごと消します。平文と判定結果が同じアイテムだと、
//! > 内訳も閾値も一緒に消えます。第2段階で「素点と重みを保存したから、平文を消したあとでも
//! > シミュレーションできる」と確認しましたよね。同じアイテムにTTLをかけると、その利点が消えます。
//!
//! そこでテーブルを分けました。
//!
//! | テーブル | 中身 | TTL |
//! |---|---|---|
//! | `action_requests` | 判定結果・内訳・そのときの閾値。**平文なし** | 無し（消さない） |
//! | `request_payloads` | 本文（平文）だけ | **あり** |
//! | `audit_logs` | 監査ログ。**追記のみ** | 無し（消せない） |
//!
//! 平文が消えても、閾値シミュレーションは動きます。
//!
//! </div>
//!
//! <div class="warning">
//!
//! <h2>【重要】TTL は時間どおりに消えません</h2>
//!
//! DynamoDB の TTL は、期限を過ぎてから**通常48時間以内**に削除されます。
//! 消えるまでのあいだ、Query や Scan の結果には出続けます。
//!
//! だから**アプリ側で期限切れを弾きます**（[`PayloadStore::get_payload`]）。
//! これが無いと、期限を過ぎた平文が読めてしまいます。
//!
//! </div>

pub mod dynamo;
pub mod memory;

pub use memory::{InMemoryStore, RequestState, RequestStore, StoredRequest};

use std::collections::HashMap;

use aws_sdk_dynamodb::Client;
use aws_sdk_dynamodb::types::{
    AttributeDefinition, AttributeValue, BillingMode, KeySchemaElement, KeyType,
    ScalarAttributeType, TimeToLiveSpecification,
};

pub mod tables {
    pub const REQUESTS: &str = "action_requests";
    /// 平文だけを置く。ここにだけ TTL を付ける。
    pub const PAYLOADS: &str = "request_payloads";
    pub const AUDIT: &str = "audit_logs";
}

/// TTL に使う属性名。DynamoDB 側にもこの名前で登録する。
pub const TTL_ATTRIBUTE: &str = "expires_at";

/// 接続を作る。
///
/// 【重要】認証情報をコードに書きません（仕様書9章）。環境変数から読みます。
/// `GATE_DYNAMO_ENDPOINT` が設定されていればそこへ繋ぎます（DynamoDB Local）。
pub async fn connect() -> Client {
    let mut config = aws_config::from_env().region(
        aws_config::meta::region::RegionProviderChain::default_provider().or_else("ap-northeast-1"),
    );
    if let Ok(endpoint) = std::env::var("GATE_DYNAMO_ENDPOINT") {
        config = config.endpoint_url(endpoint);
    }
    Client::new(&config.load().await)
}

/// テーブルを作る（無ければ）。
///
/// 【重要】本番では Terraform / CDK で作るべきものですが、
/// 第6段階はローカルで完結させる方針なので、ここで作ります。
/// 第7段階で IAM を含めた定義に移します。
pub async fn ensure_tables(client: &Client) -> Result<(), StoreError> {
    let existing = client
        .list_tables()
        .send()
        .await
        .map_err(|e| StoreError::Aws(e.to_string()))?;
    let names = existing.table_names();

    for (table, pk, sk) in [
        // 判定結果。id だけで引く。
        (tables::REQUESTS, "pk", None),
        // 平文。id だけで引く。TTL を付ける。
        (tables::PAYLOADS, "pk", None),
        // 監査ログ。要求ごとに時刻順で並ぶ。
        (tables::AUDIT, "pk", Some("sk")),
    ] {
        if names.iter().any(|n| n == table) {
            continue;
        }
        let mut builder = client
            .create_table()
            .table_name(table)
            .billing_mode(BillingMode::PayPerRequest)
            .attribute_definitions(
                AttributeDefinition::builder()
                    .attribute_name(pk)
                    .attribute_type(ScalarAttributeType::S)
                    .build()
                    .map_err(|e| StoreError::Aws(e.to_string()))?,
            )
            .key_schema(
                KeySchemaElement::builder()
                    .attribute_name(pk)
                    .key_type(KeyType::Hash)
                    .build()
                    .map_err(|e| StoreError::Aws(e.to_string()))?,
            );

        if let Some(sk) = sk {
            builder = builder
                .attribute_definitions(
                    AttributeDefinition::builder()
                        .attribute_name(sk)
                        .attribute_type(ScalarAttributeType::S)
                        .build()
                        .map_err(|e| StoreError::Aws(e.to_string()))?,
                )
                .key_schema(
                    KeySchemaElement::builder()
                        .attribute_name(sk)
                        .key_type(KeyType::Range)
                        .build()
                        .map_err(|e| StoreError::Aws(e.to_string()))?,
                );
        }

        builder
            .send()
            .await
            .map_err(|e| StoreError::Aws(e.to_string()))?;
    }

    // 【重要】TTL は平文のテーブルにだけ付ける。
    // action_requests に付けると、内訳と閾値まで一緒に消える。
    // audit_logs に付けるのは論外（消せないことが要件）。
    let ttl = client
        .describe_time_to_live()
        .table_name(tables::PAYLOADS)
        .send()
        .await
        .map_err(|e| StoreError::Aws(e.to_string()))?;
    let enabled = ttl
        .time_to_live_description()
        .and_then(|d| d.attribute_name())
        .is_some();
    if !enabled {
        client
            .update_time_to_live()
            .table_name(tables::PAYLOADS)
            .time_to_live_specification(
                TimeToLiveSpecification::builder()
                    .enabled(true)
                    .attribute_name(TTL_ATTRIBUTE)
                    .build()
                    .map_err(|e| StoreError::Aws(e.to_string()))?,
            )
            .send()
            .await
            .map_err(|e| StoreError::Aws(e.to_string()))?;
    }

    Ok(())
}

#[derive(Debug)]
pub enum StoreError {
    Aws(String),
    Encoding(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Aws(e) => write!(f, "DynamoDB: {e}"),
            Self::Encoding(e) => write!(f, "変換できません: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// 平文の置き場。
///
/// <div class="warning">
///
/// 【重要】読むときに必ず期限を確かめます。
///
/// TTL の削除は最大48時間遅れます。その間、アイテムは読めてしまいます。
/// 「期限が来たら消える」に任せると、**期限を過ぎた平文が返ります**。
///
/// </div>
pub struct PayloadStore {
    client: Client,
}

impl PayloadStore {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// 平文を置く。期限はまだ入れない（承認が終わるまで消さない）。
    pub async fn put_payload(&self, id: &str, text: &str) -> Result<(), StoreError> {
        let mut item = HashMap::new();
        item.insert("pk".to_string(), AttributeValue::S(id.to_string()));
        item.insert("text".to_string(), AttributeValue::S(text.to_string()));
        self.client
            .put_item()
            .table_name(tables::PAYLOADS)
            .set_item(Some(item))
            .send()
            .await
            .map_err(|e| StoreError::Aws(e.to_string()))?;
        Ok(())
    }

    /// 平文を読む。**期限を過ぎていれば無いものとして返す。**
    pub async fn get_payload(
        &self,
        id: &str,
        now_epoch: i64,
    ) -> Result<Option<String>, StoreError> {
        let out = self
            .client
            .get_item()
            .table_name(tables::PAYLOADS)
            .key("pk", AttributeValue::S(id.to_string()))
            .send()
            .await
            .map_err(|e| StoreError::Aws(e.to_string()))?;

        let Some(item) = out.item else {
            return Ok(None);
        };

        // 【重要】ここが無いと、TTL の削除が遅れているあいだ平文が読める。
        if let Some(AttributeValue::N(expires)) = item.get(TTL_ATTRIBUTE)
            && expires.parse::<i64>().unwrap_or(i64::MAX) <= now_epoch
        {
            return Ok(None);
        }

        Ok(item.get("text").and_then(|v| match v {
            AttributeValue::S(s) => Some(s.clone()),
            _ => None,
        }))
    }

    /// 期限を設定する。判断が下りた時点で呼ぶ。
    ///
    /// 【重要】ここで初めて時計が動き出します。承認待ちのあいだは消しません。
    /// 承認者が見るべきものが、見る前に消えては困るためです。
    pub async fn schedule_deletion(&self, id: &str, expires_at: i64) -> Result<(), StoreError> {
        self.client
            .update_item()
            .table_name(tables::PAYLOADS)
            .key("pk", AttributeValue::S(id.to_string()))
            .update_expression("SET #ttl = :t")
            .expression_attribute_names("#ttl", TTL_ATTRIBUTE)
            .expression_attribute_values(":t", AttributeValue::N(expires_at.to_string()))
            // 【重要】無いものを作らない。すでに消えているなら、そのまま。
            .condition_expression("attribute_exists(pk)")
            .send()
            .await
            .map(|_| ())
            .or_else(|e| {
                let msg = e.to_string();
                if msg.contains("ConditionalCheckFailed") {
                    Ok(()) // すでに消えている。問題ない
                } else {
                    Err(StoreError::Aws(msg))
                }
            })
    }
}
