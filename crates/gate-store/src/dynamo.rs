//! DynamoDB 版の置き場。
//!
//! <div class="warning">
//!
//! 【重要】3つのテーブルで、消し方が違います。
//!
//! - `action_requests`（判定結果）— **消しません**。閾値シミュレーションが使います
//! - `request_payloads`（平文）— 判断が下りてから24時間で**消えます**
//! - `audit_logs`（監査ログ）— **消せません**。更新も削除も呼びません
//!
//! </div>

use async_trait::async_trait;
use aws_sdk_dynamodb::Client;
use aws_sdk_dynamodb::types::AttributeValue;
use gate_core::review::{AuditEntry, Verdict};
use gate_core::score::RiskAssessment;

use crate::memory::{RequestState, RequestStore, StoredRequest};
use crate::{
    PENDING_INDEX, PENDING_KEY_ATTRIBUTE, PENDING_KEY_VALUE, PENDING_SORT_ATTRIBUTE, PayloadStore,
    VERSION_ITEM_KEY, tables,
};

/// 判断が下りてから、平文を消すまでの猶予。
///
/// 【重要】TTL の削除は**最大48時間遅れます**（AWS の仕様）。
/// 「24時間で消えます」と言い切らないこと。実際の削除はもっと遅れます。
/// だから読むときにアプリ側で弾いています（[`PayloadStore::get_payload`]）。
pub const PAYLOAD_TTL_SECONDS: i64 = 24 * 60 * 60;

/// 誰も判断しないまま放置された要求を、期限切れとして閉じるまでの時間。
///
/// 【重要】これが無いと、承認待ちのまま放置された平文が永久に残ります
/// （諏訪の指示・第6段階）。閉じて初めて TTL の時計が動き出します。
pub const ABANDON_AFTER_SECONDS: i64 = 7 * 24 * 60 * 60;

pub struct DynamoStore {
    client: Client,
    payloads: PayloadStore,
}

impl DynamoStore {
    pub fn new(client: Client) -> Self {
        Self {
            payloads: PayloadStore::new(client.clone()),
            client,
        }
    }

    /// 版番号を1つ増やす。書き込みのたびに呼ぶ。
    ///
    /// 【重要】これがあるおかげで、304 の判定が GetItem 1回で済みます。
    /// 以前は「変わっていない」と答えるために全件 Scan していました。
    async fn bump_version(&self) {
        let _ = self
            .client
            .update_item()
            .table_name(tables::REQUESTS)
            .key("pk", AttributeValue::S(VERSION_ITEM_KEY.to_string()))
            .update_expression("ADD n :one")
            .expression_attribute_values(":one", AttributeValue::N("1".to_string()))
            .send()
            .await;
    }

    fn now_epoch() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// 判定結果を JSON にして1属性へ入れる。
    ///
    /// 【重要】平文はここに入りません。入っているのは内訳・閾値・検出の要約だけです。
    fn assessment_json(a: &RiskAssessment) -> String {
        serde_json::to_string(a).unwrap_or_else(|_| "{}".to_string())
    }

    fn read_assessment(
        item: &std::collections::HashMap<String, AttributeValue>,
    ) -> Option<RiskAssessment> {
        match item.get("assessment") {
            Some(AttributeValue::S(s)) => serde_json::from_str(s).ok(),
            _ => None,
        }
    }

    fn text(item: &std::collections::HashMap<String, AttributeValue>, key: &str) -> Option<String> {
        match item.get(key) {
            Some(AttributeValue::S(s)) => Some(s.clone()),
            _ => None,
        }
    }

    /// 保存された1件を組み立てる。平文は別テーブルから、期限を見て取る。
    async fn hydrate(
        &self,
        item: std::collections::HashMap<String, AttributeValue>,
    ) -> Option<StoredRequest> {
        let id = Self::text(&item, "pk")?;
        let assessment = Self::read_assessment(&item)?;
        let now = Self::now_epoch();

        // 【重要】期限切れなら None が返る。TTL の削除待ちでも読ませない。
        let payload = self.payloads.get_payload(&id, now).await.ok().flatten();

        let state = match Self::text(&item, "status").as_deref() {
            Some("pending") | None => RequestState::Pending,
            _ => RequestState::Settled {
                verdict: match Self::text(&item, "verdict").as_deref() {
                    Some("approved") => Verdict::Approved,
                    Some("approved_with_masking") => Verdict::ApprovedWithMasking,
                    Some("expired") => Verdict::Expired,
                    _ => Verdict::Rejected,
                },
                returned_payload: Self::text(&item, "returned_payload"),
                reviewer: Self::text(&item, "reviewer").unwrap_or_default(),
                at: Self::text(&item, "decided_at").unwrap_or_default(),
            },
        };

        // マスキング計画は保存しません。平文が消えたあとには意味が無いためです。
        // 承認画面で要るときは、平文から作り直します。
        let mask_plan = match &payload {
            Some(text) => {
                let scan =
                    gate_core::detect::scan(text, &gate_core::detect::DetectConfig::default());
                gate_core::mask::plan(text, &scan)
            }
            None => gate_core::mask::MaskPlan::default(),
        };

        Some(StoredRequest {
            id,
            agent_id: Self::text(&item, "agent_id").unwrap_or_default(),
            created_at: Self::text(&item, "created_at").unwrap_or_default(),
            assessment,
            payload_available: payload.is_some(),
            payload: payload.unwrap_or_default(),
            mask_plan,
            state,
        })
    }

    /// 全件を取る。**閾値シミュレーションと履歴だけが使います。**
    ///
    /// 【重要】ここは Scan のままです。承認待ちの一覧（毎秒のポーリング）は
    /// 索引から引くので、Scan が走るのは人が画面を操作したときだけになりました。
    /// シミュレーションは過去の全件を見る操作なので、Scan が筋です。
    ///
    /// 版番号の項目（`#version`）は判定結果を持たないので、hydrate が弾きます。
    async fn scan_requests(&self) -> Vec<StoredRequest> {
        let out = match self.client.scan().table_name(tables::REQUESTS).send().await {
            Ok(out) => out,
            Err(_) => return Vec::new(),
        };
        let mut requests = Vec::new();
        for item in out.items.unwrap_or_default() {
            if let Some(r) = self.hydrate(item).await {
                requests.push(r);
            }
        }
        requests.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        requests
    }

    /// 放置された承認待ちを、期限切れとして閉じる。
    ///
    /// <div class="warning">
    ///
    /// 【重要】諏訪の指示（第6段階）:
    ///
    /// > 「承認完了後24時間」だと、承認待ちのまま放置された平文が永久に残ります。
    /// > 一定期間で「期限切れ」として閉じて、そこからTTLを開始する形にしてください。
    ///
    /// 閉じたことは監査ログにも残します。**判断しなかったことも記録**です。
    ///
    /// </div>
    pub async fn expire_abandoned(&self, older_than_secs: i64, now_rfc3339: &str) -> usize {
        let now = Self::now_epoch();
        let mut closed = 0;

        for r in self.scan_requests().await {
            if !r.state.is_pending() {
                continue;
            }
            let created = chrono_epoch(&r.created_at);
            if now - created < older_than_secs {
                continue;
            }
            let state = RequestState::Settled {
                verdict: Verdict::Expired,
                returned_payload: None,
                reviewer: gate_core::review::Reviewer::System.describe(),
                at: now_rfc3339.to_string(),
            };
            if self.settle(&r.id, state).await.is_some() {
                self.append_audit(AuditEntry::new(
                    &r.id,
                    now_rfc3339,
                    gate_core::review::Reviewer::System,
                    Verdict::Expired,
                    &r.assessment,
                ))
                .await;
                closed += 1;
            }
        }
        closed
    }
}

/// RFC3339 の文字列をエポック秒に。読めなければ 0（＝すぐ期限切れにはしない側へ倒す）。
fn chrono_epoch(rfc3339: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|d| d.timestamp())
        .unwrap_or_else(|_| {
            // 【重要】読めない時刻を「大昔」と解釈すると、
            // 壊れた行が片っ端から期限切れにされます。安全側に倒して「いま」とみなす。
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        })
}

#[async_trait]
impl RequestStore for DynamoStore {
    async fn put(&self, request: StoredRequest) {
        // ① 平文は別テーブルへ。ここにだけ TTL が付く。
        let _ = self
            .payloads
            .put_payload(&request.id, &request.payload)
            .await;

        // ② 判定結果。平文は入れない。
        let mut builder = self
            .client
            .put_item()
            .table_name(tables::REQUESTS)
            .item("pk", AttributeValue::S(request.id.clone()))
            .item("agent_id", AttributeValue::S(request.agent_id.clone()))
            .item("created_at", AttributeValue::S(request.created_at.clone()))
            .item(
                "assessment",
                AttributeValue::S(Self::assessment_json(&request.assessment)),
            );

        builder = match &request.state {
            RequestState::Pending => builder
                .item("status", AttributeValue::S("pending".into()))
                // 索引に載せる。判断が下りたら消す。
                .item(
                    PENDING_KEY_ATTRIBUTE,
                    AttributeValue::S(PENDING_KEY_VALUE.to_string()),
                )
                .item(
                    PENDING_SORT_ATTRIBUTE,
                    AttributeValue::S(request.created_at.clone()),
                ),
            RequestState::Settled {
                verdict,
                returned_payload,
                reviewer,
                at,
            } => {
                let mut b = builder
                    .item("status", AttributeValue::S("settled".into()))
                    .item("verdict", AttributeValue::S(verdict_key(*verdict)))
                    .item("reviewer", AttributeValue::S(reviewer.clone()))
                    .item("decided_at", AttributeValue::S(at.clone()));
                if let Some(p) = returned_payload {
                    b = b.item("returned_payload", AttributeValue::S(p.clone()));
                }
                b
            }
        };
        let _ = builder.send().await;
        self.bump_version().await;

        // 自動承認は、その場で判断が済んでいる。平文の時計をここで動かす。
        if !request.state.is_pending() {
            let _ = self
                .payloads
                .schedule_deletion(&request.id, Self::now_epoch() + PAYLOAD_TTL_SECONDS)
                .await;
        }
    }

    async fn get(&self, id: &str) -> Option<StoredRequest> {
        let out = self
            .client
            .get_item()
            .table_name(tables::REQUESTS)
            .key("pk", AttributeValue::S(id.to_string()))
            .send()
            .await
            .ok()?;
        self.hydrate(out.item?).await
    }

    async fn pending(&self) -> Vec<StoredRequest> {
        // 【重要】Scan ではなく索引から引きます。
        //
        // 承認待ちだけが載る索引（sparse index）なので、承認済みが何万件たまっても
        // 一覧の費用は増えません。以前は全件 Scan していたため、
        // 保存件数に比例して読み取り費用が増える構造でした（500件で月$17）。
        let out = self
            .client
            .query()
            .table_name(tables::REQUESTS)
            .index_name(PENDING_INDEX)
            .key_condition_expression("#k = :pending")
            .expression_attribute_names("#k", PENDING_KEY_ATTRIBUTE)
            .expression_attribute_values(
                ":pending",
                AttributeValue::S(PENDING_KEY_VALUE.to_string()),
            )
            .send()
            .await;

        let Ok(out) = out else {
            return Vec::new();
        };
        let mut requests = Vec::new();
        for item in out.items.unwrap_or_default() {
            if let Some(r) = self.hydrate(item).await {
                requests.push(r);
            }
        }
        requests.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        requests
    }

    async fn all(&self) -> Vec<StoredRequest> {
        let mut all = self.scan_requests().await;
        all.reverse();
        all
    }

    async fn settle(&self, id: &str, state: RequestState) -> Option<StoredRequest> {
        let RequestState::Settled {
            verdict,
            returned_payload,
            reviewer,
            at,
        } = &state
        else {
            return None;
        };

        let mut update = self
            .client
            .update_item()
            .table_name(tables::REQUESTS)
            .key("pk", AttributeValue::S(id.to_string()))
            // 【重要】判断が下りたら索引のキーを消す。承認待ちの索引から落ちる。
            .update_expression(
                "SET #s = :settled, verdict = :v, reviewer = :r, decided_at = :d\
                 , returned_payload = :p REMOVE #pending",
            )
            .expression_attribute_names("#pending", PENDING_KEY_ATTRIBUTE)
            .expression_attribute_names("#s", "status")
            .expression_attribute_values(":settled", AttributeValue::S("settled".into()))
            .expression_attribute_values(":v", AttributeValue::S(verdict_key(*verdict)))
            .expression_attribute_values(":r", AttributeValue::S(reviewer.clone()))
            .expression_attribute_values(":d", AttributeValue::S(at.clone()))
            .expression_attribute_values(
                ":p",
                match returned_payload {
                    Some(p) => AttributeValue::S(p.clone()),
                    None => AttributeValue::Null(true),
                },
            );

        // 【重要】二重承認を弾くのは、アプリの if 文ではなく DynamoDB の条件式。
        // 同時に2人が押しても、通るのは片方だけになる。
        update = update
            .condition_expression("attribute_exists(pk) AND #s = :pending")
            .expression_attribute_values(":pending", AttributeValue::S("pending".into()));

        update.send().await.ok()?;
        self.bump_version().await;

        // 判断が下りた。ここから平文の時計が動き出す。
        let _ = self
            .payloads
            .schedule_deletion(id, Self::now_epoch() + PAYLOAD_TTL_SECONDS)
            .await;

        self.get(id).await
    }

    async fn append_audit(&self, entry: AuditEntry) {
        // 【重要】put_item だけ。update も delete も呼びません。
        // 同じキーへの上書きも防ぎます（条件式）。第7段階では IAM でも縛ります。
        let json = serde_json::to_string(&entry).unwrap_or_default();
        let _ = self
            .client
            .put_item()
            .table_name(tables::AUDIT)
            .item("pk", AttributeValue::S(entry.request_id.clone()))
            .item(
                "sk",
                AttributeValue::S(format!("{}#{}", entry.at, verdict_key(entry.verdict))),
            )
            .item("entry", AttributeValue::S(json))
            .condition_expression("attribute_not_exists(pk) AND attribute_not_exists(sk)")
            .send()
            .await;
    }

    async fn audit_log(&self) -> Vec<AuditEntry> {
        let Ok(out) = self.client.scan().table_name(tables::AUDIT).send().await else {
            return Vec::new();
        };
        let mut entries: Vec<AuditEntry> = out
            .items
            .unwrap_or_default()
            .iter()
            .filter_map(|item| match item.get("entry") {
                Some(AttributeValue::S(s)) => serde_json::from_str(s).ok(),
                _ => None,
            })
            .collect();
        entries.sort_by(|a: &AuditEntry, b: &AuditEntry| a.at.cmp(&b.at));
        entries
    }

    async fn version(&self) -> u64 {
        // 【重要】1件だけ取ります（0.5 RRU）。
        //
        // 以前はここで承認待ちを全件 Scan していました。ETag で 304 を返しても、
        // 「変わっていない」と確かめるために毎回 Scan していたので、
        // **転送量は減っても読み取り費用は減っていませんでした**。
        // 見積もりを作るまで気づけませんでした（メモリ版では番号がタダだったため）。
        let out = self
            .client
            .get_item()
            .table_name(tables::REQUESTS)
            .key("pk", AttributeValue::S(VERSION_ITEM_KEY.to_string()))
            .send()
            .await;
        match out {
            Ok(o) => o
                .item
                .and_then(|i| match i.get("n") {
                    Some(AttributeValue::N(n)) => n.parse::<u64>().ok(),
                    _ => None,
                })
                .unwrap_or(0),
            Err(_) => 0,
        }
    }
}

fn verdict_key(v: Verdict) -> String {
    match v {
        Verdict::Approved => "approved",
        Verdict::ApprovedWithMasking => "approved_with_masking",
        Verdict::Rejected => "rejected",
        Verdict::Expired => "expired",
    }
    .to_string()
}
