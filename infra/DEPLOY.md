# デプロイ手順（諏訪さんの手元で実行）

**Claude Code は AWS に触れていません。** コマンドの用意までがこちら、実行は手元でお願いします。
認証情報を渡さないので、事故の範囲が読める状態になります。

実行前に、**何が走るか**を上から順に読めるようにしてあります。危険なもの（課金・削除）には印を付けました。

```
所要時間  15〜20分（＋動作確認）
費用      月 $0.1 未満の見込み（README「AWS の想定月額」参照）
止め方    このファイルの最後。★その日のうちに止めます★
```

**0番から順に、飛ばさずに実行してください。** 予算アラートを先に置いてあるのは、
デプロイしてから設定すると、その間に起きた事故に気づけないためです。

**証跡は `infra/evidence/` に保存してください。** 各手順に `> infra/evidence/...` を入れてあります。
あとから「本当に消したのか」「本当に消せない設定なのか」を確かめられる形にするためです。

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

### ⚠️ この URL は、知っている人なら誰でも叩けます

`--enable-function-url` が作る Function URL は、**認証タイプが `NONE`** です。
このシステムは認証を意図的に入れていないので（README「認証は、意図的に実装していません」）、
**URL を知っていれば、誰でも要求を投げられ、承認も拒否もできます。**

そう分かったうえで動かす、という前提です。実害を小さくしている条件は3つ。

- **投入するのは架空のデータだけ**（仕様書9章。実在の個人情報は入れない）
- **予算アラートが先に入っている**（手順0）
- **その日のうちに止める**（手順7）

URL は推測しにくい文字列ですが、**推測されにくいだけで、秘密ではありません。**

> **【重要】スクリーンショットと出力に URL が写り込みます。**
> README に貼るとき、`infra/evidence/` を共有するときは、URL を伏せてください。
> リポジトリを公開したあとで気づいても、履歴からは消えません。

```bash
# 確認: AuthType が NONE であること（そう表示されるのが想定どおり）
aws lambda get-function-url-config --function-name gate-api \
  --query '{url:FunctionUrl, auth:AuthType}'
```

**IAM 認証に変えることもできます**が、その場合は curl も画面も SigV4 で署名する必要があり、
README に載せている「curl でそのまま叩ける」形が崩れます。
短時間で止める前提なら、`NONE` のままで進めるほうが筋が通ると考えています。

```bash
# もし IAM 認証に変える場合（curl は署名が要るようになります）
aws lambda update-function-url-config --function-name gate-api --auth-type AWS_IAM
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

### 監査ログが消せないことを、実行せずに証明する

**IAM Policy Simulator を使います。** 実際に消しに行く必要がないので安全で、
出力をそのまま証跡として残せます。

```bash
ROLE_ARN=arn:aws:iam::$ACCOUNT_ID:role/gate-api-role
AUDIT_ARN=arn:aws:dynamodb:ap-northeast-1:$ACCOUNT_ID:table/audit_logs

aws iam simulate-principal-policy \
  --policy-source-arn "$ROLE_ARN" \
  --action-names dynamodb:DeleteItem dynamodb:UpdateItem dynamodb:BatchWriteItem \
                 dynamodb:PutItem dynamodb:Query \
  --resource-arns "$AUDIT_ARN" \
  | tee infra/evidence/iam-simulate-audit.json \
  | python3 -c "
import json, sys
for r in json.load(sys.stdin)['EvaluationResults']:
    print(f\"{r['EvalActionName']:<28} {r['EvalDecision']}\")
"
```

**期待する出力はこれです。**

```
dynamodb:DeleteItem          explicitDeny     ← 消せない
dynamodb:UpdateItem          explicitDeny     ← 直せない
dynamodb:BatchWriteItem      explicitDeny     ← まとめても消せない
dynamodb:PutItem             allowed          ← 追記はできる
dynamodb:Query               allowed          ← 読める
```

`explicitDeny` は「Deny が明示的に効いている」という意味です。
**`BatchWriteItem` をここに含めているのが要点です。** `DeleteItem` だけを拒否しても、
`BatchWriteItem` の中に `DeleteRequest` を入れれば消せてしまいます。

**面談で「本当に消せないんですか」と聞かれたら、この出力を見せるのがいちばん早い**です。

あわせて、アプリ側にも削除の経路が無いことを残します。

```bash
grep -rn "delete_item\|DeleteItem" crates/ \
  > infra/evidence/no-delete-in-code.txt 2>&1 \
  || echo "コードに削除の呼び出しはありません" > infra/evidence/no-delete-in-code.txt
cat infra/evidence/no-delete-in-code.txt
```

### 動作確認の出力を残す

```bash
{
  echo "=== health ==="; curl -s "${URL}api/health"
  echo; echo "=== policy ==="; curl -s "${URL}api/policy"
  echo; echo "=== queue ==="; curl -s "${URL}api/queue"
  echo; echo "=== audit ==="; curl -s "${URL}api/audit"
} > infra/evidence/aws-run.txt
```

**スクリーンショット**（ダッシュボードの承認待ち・内訳・シミュレーション）も
`infra/evidence/` に置いてください。第8段階で README に貼ります。

---

## 7. 止める（💥 **その日のうちに実行**）

**動作確認 → スクリーンショット → README に反映 → 削除。** 運用はしません。
ここまで来たら、間を空けずに流してください。

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

# 【重要】消え残りの確認。出力を保存する。
#
# 「消したつもりで残っている」が、この手のいちばんよくある事故です。
# 出力があれば、あとから確かめられます。
{
  echo "=== 実行日時 ==="; date
  echo; echo "=== DynamoDB のテーブル（gate 関連が無いこと）==="
  aws dynamodb list-tables
  echo; echo "=== ロググループ（空であること）==="
  aws logs describe-log-groups --log-group-name-prefix /aws/lambda/gate \
    --query 'logGroups[].logGroupName'
  echo; echo "=== Lambda 関数（空であること）==="
  aws lambda list-functions \
    --query 'Functions[?starts_with(FunctionName, `gate`)].FunctionName'
  echo; echo "=== IAM ロール（空であること）==="
  aws iam list-roles --query 'Roles[?starts_with(RoleName, `gate`)].RoleName'
  echo; echo "=== 予算アラート（残してよい）==="
  aws budgets describe-budgets --account-id "$ACCOUNT_ID" --query 'Budgets[].BudgetName'
} > infra/evidence/teardown.txt

cat infra/evidence/teardown.txt
```

**期待する形はこれです。**

```
=== DynamoDB のテーブル（gate 関連が無いこと）===
{ "TableNames": [] }
=== ロググループ（空であること）===
[]
=== Lambda 関数（空であること）===
[]
=== IAM ロール（空であること）===
[]
=== 予算アラート（残してよい）===
[ "ai-action-gate" ]
```

**予算アラートは残して構いません**（無料）。消し忘れに気づく最後の砦になります。

---

## 実行の記録（チェックリスト）

| | 手順 | 証跡 |
|---|---|---|
| ☐ | 0. 予算アラート（ACTUAL 50% ＋ FORECASTED 80%） | `describe-budgets` の出力 |
| ☐ | 1〜2. ビルド | — |
| ☐ | 3. DynamoDB 3テーブル ＋ TTL ＋ PITR | — |
| ☐ | 4. IAM ロール（Deny つき） | — |
| ☐ | 5. Lambda ＋ **ログ保持7日** | — |
| ☐ | 5. **Function URL が `NONE`（誰でも叩ける）と確認** | `get-function-url-config` の出力 |
| ☐ | 6. 動作確認 | `aws-run.txt` |
| ☐ | 6. **IAM Simulator で Deny を証明** | `iam-simulate-audit.json` |
| ☐ | 6. スクリーンショット | `*.png` |
| ☐ | 7. **その日のうちに削除** | `teardown.txt` |
