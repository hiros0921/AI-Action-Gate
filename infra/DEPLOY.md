# デプロイ手順（諏訪さんの手元で実行）

**Claude Code は AWS に触れていません。** コマンドの用意までがこちら、実行は手元でお願いします。
認証情報を渡さないので、事故の範囲が読める状態になります。

実行前に、**何が走るか**を上から順に読めるようにしてあります。危険なもの（課金・削除）には印を付けました。

```
所要時間  15〜20分
費用      月 $0.1 未満の見込み（README「AWS の想定月額」参照）
止め方    このファイルの最後。作業当日に止めます
```

---

## 0. 先に予算アラート（課金が始まる前に）

**デプロイより先にこれをやります。** 事故に気づける状態を作ってから始めます。

```bash
ACCOUNT_ID=$(aws sts get-caller-identity --query Account --output text)
EMAIL=<通知を受け取るメールアドレス>

cat > /tmp/budget.json <<EOF
{
  "BudgetName": "ai-action-gate",
  "BudgetLimit": { "Amount": "10", "Unit": "USD" },
  "TimeUnit": "MONTHLY",
  "BudgetType": "COST"
}
EOF

# 【重要】ACTUAL（実績）と FORECASTED（予測）の両方を入れる。
# ACTUAL だけだと、使ってしまってから通知が来ます。
# FORECASTED は「月末にこの額になりそう」の時点で鳴るので、早く気づけます。
cat > /tmp/notifications.json <<EOF
[
  {
    "Notification": {
      "NotificationType": "ACTUAL",
      "ComparisonOperator": "GREATER_THAN",
      "Threshold": 50,
      "ThresholdType": "PERCENTAGE"
    },
    "Subscribers": [{ "SubscriptionType": "EMAIL", "Address": "$EMAIL" }]
  },
  {
    "Notification": {
      "NotificationType": "FORECASTED",
      "ComparisonOperator": "GREATER_THAN",
      "Threshold": 80,
      "ThresholdType": "PERCENTAGE"
    },
    "Subscribers": [{ "SubscriptionType": "EMAIL", "Address": "$EMAIL" }]
  }
]
EOF

aws budgets create-budget \
  --account-id "$ACCOUNT_ID" \
  --budget file:///tmp/budget.json \
  --notifications-with-subscribers file:///tmp/notifications.json

# 確認
aws budgets describe-budgets --account-id "$ACCOUNT_ID" --query 'Budgets[].BudgetName'
```

実績が $5（50%）で1回、予測が $8（80%）に届いた時点でもう1回鳴ります。

---

## 1. 必要なもの

```bash
brew install zig                       # クロスコンパイル用
cargo install cargo-lambda             # まだなら
aws sts get-caller-identity            # 認証情報が通っているか確認
```

---

## 2. ビルド（AWSには触りません）

```bash
cd /Users/suwahiroyuki/Desktop/zed/AI-Action-Gate
cargo lambda build --release --arm64 -p gate-api
ls -lh target/lambda/gate-api/bootstrap
```

**arm64（Graviton）にするのは、x86 より約20%安く、Rust では移植の手間が無いためです。**

---

## 3. DynamoDB のテーブル（💰 課金対象）

```bash
REGION=ap-northeast-1

# 判定結果。承認待ちを引く索引つき（Scan を避けるため）
aws dynamodb create-table --region $REGION \
  --table-name action_requests \
  --billing-mode PAY_PER_REQUEST \
  --attribute-definitions \
      AttributeName=pk,AttributeType=S \
      AttributeName=pending_key,AttributeType=S \
      AttributeName=created_at,AttributeType=S \
  --key-schema AttributeName=pk,KeyType=HASH \
  --global-secondary-indexes '[{
      "IndexName": "pending_index",
      "KeySchema": [
        {"AttributeName":"pending_key","KeyType":"HASH"},
        {"AttributeName":"created_at","KeyType":"RANGE"}
      ],
      "Projection": {"ProjectionType":"ALL"}
    }]'

# 平文。ここにだけ TTL を付ける
aws dynamodb create-table --region $REGION \
  --table-name request_payloads \
  --billing-mode PAY_PER_REQUEST \
  --attribute-definitions AttributeName=pk,AttributeType=S \
  --key-schema AttributeName=pk,KeyType=HASH

aws dynamodb update-time-to-live --region $REGION \
  --table-name request_payloads \
  --time-to-live-specification 'Enabled=true,AttributeName=expires_at'

# 監査ログ。TTL は付けない（消えては困る）
aws dynamodb create-table --region $REGION \
  --table-name audit_logs \
  --billing-mode PAY_PER_REQUEST \
  --attribute-definitions AttributeName=pk,AttributeType=S AttributeName=sk,AttributeType=S \
  --key-schema AttributeName=pk,KeyType=HASH AttributeName=sk,KeyType=RANGE

# 【重要】監査ログは戻せるようにしておく。消せないことと、壊れたときに戻せることは別。
aws dynamodb update-continuous-backups --region $REGION \
  --table-name audit_logs \
  --point-in-time-recovery-specification PointInTimeRecoveryEnabled=true
```

---

## 4. IAM ロール（監査ログを守る本体）

```bash
ACCOUNT_ID=$(aws sts get-caller-identity --query Account --output text)

cat > /tmp/trust.json <<'EOF'
{
  "Version": "2012-10-17",
  "Statement": [{
    "Effect": "Allow",
    "Principal": { "Service": "lambda.amazonaws.com" },
    "Action": "sts:AssumeRole"
  }]
}
EOF

aws iam create-role --role-name gate-api-role \
  --assume-role-policy-document file:///tmp/trust.json

# 説明用の Comment キーを外してから渡す
python3 -c "
import json
p = json.load(open('infra/audit-append-only-policy.json'))
p.pop('Comment', None)
print(json.dumps(p).replace('ACCOUNT_ID', '$ACCOUNT_ID'))
" > /tmp/policy.json

aws iam put-role-policy --role-name gate-api-role \
  --policy-name gate-api-policy --file-document file:///tmp/policy.json 2>/dev/null || \
aws iam put-role-policy --role-name gate-api-role \
  --policy-name gate-api-policy --policy-document file:///tmp/policy.json
```

**このポリシーの要点は Deny です。** `audit_logs` への `UpdateItem` / `DeleteItem` /
`BatchWriteItem` を明示的に拒否します。Deny は Allow より強いので、あとから広い権限を足しても抜けません。
`BatchWriteItem` を落とすと、その中に `DeleteRequest` を入れて消せてしまいます。

---

## 5. Lambda（💰 課金対象）

```bash
cargo lambda deploy gate-api \
  --iam-role arn:aws:iam::$ACCOUNT_ID:role/gate-api-role \
  --env-var GATE_STORE=dynamodb \
  --env-var GATE_POLL_MS=3000 \
  --env-var RUST_LOG=info \
  --memory 128 \
  --timeout 10 \
  --enable-function-url
```

**【重要】ログの保持期間を必ず設定します。既定は無期限で、静かに溜まり続けます。**

```bash
# 関数を一度でも呼ぶとロググループができる。できてから設定する
aws logs put-retention-policy \
  --log-group-name /aws/lambda/gate-api --retention-in-days 7
aws logs describe-log-groups --log-group-name-prefix /aws/lambda/gate-api \
  --query 'logGroups[].{name:logGroupName,days:retentionInDays}'
```

---

## 6. 動作確認

```bash
URL=$(aws lambda get-function-url-config --function-name gate-api --query FunctionUrl --output text)

curl -s "${URL}api/health"
curl -s "${URL}api/policy"

# LOW（自動承認される）
curl -s -X POST "${URL}api/requests" -H 'content-type: application/json' -d '{
  "agent_id":"agent-001","action":"read","destination":"internal","data_class":"public",
  "payload":{"text":"倉庫Aの在庫一覧を確認します。"}}'

# HIGH（承認待ち＋マスキング案）※本文はすべて架空
curl -s -X POST "${URL}api/requests" -H 'content-type: application/json' -d '{
  "agent_id":"agent-001","action":"send_external","destination":"external_ai",
  "data_class":"personal_information",
  "payload":{"text":"山田太郎さん（1980年3月15日生・090-1234-5678）の検査結果について"}}'

curl -s "${URL}api/queue"
```

承認は `X-Approver-Id` を付けて。ダッシュボードから触る場合は、
`dashboard/vite.config.js` の proxy 先をこの URL に変えて `npm run dev`。

**【重要】監査ログが本当に消せないことを、ここで確かめます。**

```bash
# 拒否されること（AccessDeniedException が出れば正しい）
aws dynamodb delete-item --table-name audit_logs \
  --key '{"pk":{"S":"req-xxxx"},"sk":{"S":"2026-01-01T00:00:00Z#approved"}}'
```

このコマンドは**あなたの権限**で走るので、Lambda ロールの Deny は効きません。
正しく確かめるには、Lambda から delete を呼ぶ経路が無いこと（コードに `delete_item` が
1つも無いこと）と、ロールのポリシーに Deny が入っていることの両方を見せます。

```bash
grep -rn "delete_item\|DeleteItem" crates/ || echo "コードに削除の呼び出しはありません"
aws iam get-role-policy --role-name gate-api-role --policy-name gate-api-policy \
  --query 'PolicyDocument.Statement[?Effect==`Deny`]'
```

---

## 7. 止める（💥 作業当日に実行）

```bash
API_ID=$(aws lambda get-function-url-config --function-name gate-api --query FunctionUrl --output text)

aws lambda delete-function-url-config --function-name gate-api
aws lambda delete-function --function-name gate-api

aws dynamodb delete-table --table-name action_requests
aws dynamodb delete-table --table-name request_payloads
aws dynamodb delete-table --table-name audit_logs

# 【重要】Lambda を消してもロググループは残ります。消し忘れの定番。
aws logs delete-log-group --log-group-name /aws/lambda/gate-api

aws iam delete-role-policy --role-name gate-api-role --policy-name gate-api-policy
aws iam delete-role --role-name gate-api-role

# 消え残りの確認
aws dynamodb list-tables
aws logs describe-log-groups --log-group-name-prefix /aws/lambda/gate
aws lambda list-functions --query 'Functions[?starts_with(FunctionName, `gate`)].FunctionName'
```

**予算アラートは残して構いません**（無料）。消し忘れに気づく最後の砦になります。
