# Terraform 構成

`infra/DEPLOY.md` の手動手順（AWS CLI 20コマンド超）を、コードにしたもの。
**同じものが再現でき、`destroy` で確実に消える**状態にするのが目的。

```
検証済み   tofu validate / tofu fmt を通過（OpenTofu v1.12.6）
適用済み   2026-09-03〜04 に apply → 疎通 200 → IAM Simulator で Deny 証明 → destroy
証跡       infra/evidence/terraform-run.txt
```

---

## 何を作るか

```
aws_budgets_budget          ai-action-gate（$10・ACTUAL 50% ＋ FORECASTED 80%）
aws_dynamodb_table          action_requests（GSI: pending_index）
aws_dynamodb_table          request_payloads（TTL: expires_at）
aws_dynamodb_table          audit_logs（TTL なし・PITR 有効）
aws_iam_role                gate-api-role
aws_iam_role_policy         gate-api-policy（Deny 5件を含む）
aws_cloudwatch_log_group    /aws/lambda/gate-api（保持7日）
aws_lambda_function         gate-api（arm64・128MB・10秒）
aws_lambda_function_url     認証なし
aws_lambda_permission       InvokeFunctionUrl（権限①）
terraform_data + local-exec  InvokeFunction を URL 経由に限定（権限②）
```

権限②だけ `aws_lambda_permission` ではなく CLI 呼び出しになっている理由は
`lambda.tf` のコメントに書いてある（プロバイダに `--invoked-via-function-url`
相当の属性が無い。実測 2026-09-03）。

---

## 手動手順から変えた3点

**1. アカウントIDがコードに現れない**

手動手順では `ACCOUNT_ID` を文字列置換していたため、証跡ファイルに12桁が残り、
`git add` の前に `sed` で伏せる作業が必要だった。
`data.aws_caller_identity` で解決するので、コードにも `.tfvars` にも現れない。

**2. ロググループを Lambda より先に作る**

Lambda は初回実行時に `/aws/lambda/gate-api` を自分で作る。そのとき保持期間は
**無期限**になる。手動手順では「関数を一度でも呼ぶとロググループができる。
できてから設定する」という順序だった。忘れると静かに溜まり続ける。

Terraform では先に作り、`depends_on` で Lambda を後ろに置く。
逆順にすると `ResourceAlreadyExistsException` で落ちるので、依存の宣言が要る。

**3. 予算アラートは destroy されない**

`lifecycle { prevent_destroy = true }` を付けてある。無料で、消し忘れに
気づく最後の砦になるため。

---

## 実測の記録（2026-09-03〜04）

最小権限の IAM ユーザ（`gate-deployer`）のまま apply した。
**`AdministratorAccess` は使っていない。** 止まった箇所を順に記す。

### 権限不足で止まった回数：4回

| # | 止まったアクション | 何をしていたか | 対処 |
|---|---|---|---|
| 1 | `budgets:TagResource` | 予算の作成 | **権限を足さず、`default_tags` を外した**。タグの目的（消し忘れ探し）は state が既に果たしているため |
| 2 | `budgets:ListTagsForResource` | 8月の手動作成分を `import` | 権限を追加 |
| 3 | `iam:ListAttachedRolePolicies` | ロール作成直後の読み戻し | 権限を追加。`ListRolePolicies`（インライン）とは別物 |
| 4 | `lambda:ListVersionsByFunction` | 関数作成直後の読み戻し | 権限を追加 |

**4回のうち3回が「読み戻し」で止まっている。** Terraform は宣言的なので、
作った直後に必ず読み直して state と突き合わせる。CLI は書きっぱなしで済む。
**この差が、CLI 手順を IaC に移すときに最初に踏む壁だった。**

```
CLI 手順で必要だった権限     52
Terraform で必要だった権限   74（+22）
  うち読み取り（List / Get）  21
  うち書き込み                1（logs:CreateLogGroup。ロググループを先に作るため）
```

### プロバイダの機能不足：1回

権限②（`lambda:InvokeFunction` を URL 経由に限定）は `aws_lambda_permission`
で書けなかった。`function_url_auth_type` を付けたら AWS に拒否された:

```
InvalidParameterValueException:
FunctionUrlAuthType is only supported for lambda:InvokeFunctionUrl action
```

CLI の `--invoked-via-function-url` に相当する属性が、プロバイダ 5.100.0 に無い
（`tofu providers schema -json` で確認）。`terraform_data` + `local-exec` で
CLI を直接呼ぶ形にした。詳細は `lambda.tf` のコメント。

### 予想と違ったこと：1件

`aws_lambda_function_url` を `NONE` で作ると、プロバイダが
`FunctionURLAllowPublicAccess` という statement を**自動で追加する**。
手動手順の権限①をそのまま書いた `aws_lambda_permission.function_url_invoke` は
重複だった。害は無いので残してある（`lambda.tf` 参照）。

### 運用上の失敗：1回

`deployer-policy.json` は説明用コメント入りで、IAM に貼る前にコメントを
外す運用にしていた。**2つを取り違えて、コメント入りのほうを貼った。**
IAM は `Comment` キーを拒否するので保存できず、「権限を足したはずなのに
効かない」状態が2往復続いた。

説明を残す版と投入する版を分ける設計自体は正しいが、**分岐が人の手に
あると事故になる。** 次回は生成を1コマンドにする。

### 結果

```
apply          10 リソース（予算は import）
疎通           HTTP 200
Deny の証明    IAM Simulator で 5 アクションが explicitDeny、2 が allowed
               → 8月の手動構成と同じ効き方
destroy        10 リソース削除（AWS 9 + terraform_data 1）。予算アラートのみ残す
               tofu state list → aws_budgets_budget.ai_action_gate の1件
```

---

## 使い方

### 0. 予算アラートだけ先に作る

**【重要】他のどのリソースよりも先に。** デプロイしてから設定すると、
その間に起きた課金事故に気づけない。

```bash
cd infra/terraform
cp terraform.tfvars.example terraform.tfvars   # 通知先メールを書く
tofu init
tofu apply -target=aws_budgets_budget.ai_action_gate
```

### 1. ビルド（Terraform の外）

```bash
cd ../..
cargo lambda build --release --arm64 -p gate-api
ls -lh target/lambda/gate-api/bootstrap
```

Terraform はビルドしない。ビルドは cargo の仕事で、配置が Terraform の仕事。

### 2. 残りを作る

```bash
cd infra/terraform
tofu plan      # 何が作られるか、先に読む
tofu apply
```

### 3. 動作確認

```bash
URL=$(tofu output -raw function_url)
curl -s "${URL}api/health"
```

`function_url` は `sensitive = true` にしてあるので、apply の最後には表示されない。
必要なときだけ `-raw` で取る。

### 4. 監査ログが消せないことを、実行せずに証明する

```bash
ROLE_ARN=$(tofu output -raw role_arn)
AUDIT_ARN=$(tofu output -raw audit_logs_table_arn)

aws iam simulate-principal-policy \
  --policy-source-arn "$ROLE_ARN" \
  --action-names dynamodb:DeleteItem dynamodb:UpdateItem dynamodb:BatchWriteItem \
                 dynamodb:PutItem dynamodb:Query \
  --resource-arns "$AUDIT_ARN"
```

期待する出力:

```
dynamodb:DeleteItem          explicitDeny     ← 消せない
dynamodb:UpdateItem          explicitDeny     ← 直せない
dynamodb:BatchWriteItem      explicitDeny     ← まとめても消せない
dynamodb:PutItem             allowed          ← 追記はできる
dynamodb:Query               allowed          ← 読める
```

### 5. その日のうちに消す

```bash
tofu destroy -exclude="aws_budgets_budget.ai_action_gate"
```

**【実測 2026-09-04】`-exclude` が要る。** 最初は `tofu destroy` だけで書いていた。
`prevent_destroy = true` を「その1件を飛ばす」と思っていたが、実際は
**destroy 全体を止める**:

```
Error: Resource instance cannot be destroyed
Resource instance aws_budgets_budget.ai_action_gate has prevent_destroy set,
but the plan calls for it to be destroyed.
```

`prevent_destroy` は事故防止の壁で、回避手段ではない。回避は `-exclude` で
明示する（エラーメッセージがそのまま教えてくれる）。

手動手順では削除コマンドを9本並べ、消し残りを `list-tables` などで
目視確認していた。`destroy` は state に載っているものを消すので、
**作ったものと消したものが一致する**（消し忘れの定番だったロググループも含む）。

予算アラートを消す場合のみ、`budget.tf` の `prevent_destroy` を `false` に
してから:

```bash
tofu destroy -target=aws_budgets_budget.ai_action_gate
```

---

## Terraform と OpenTofu

このディレクトリは両方で動く。検証は OpenTofu で行った
（Terraform は 2023年に BUSL へライセンス変更され、Homebrew core から外れたため）。

```bash
tofu init && tofu validate        # OpenTofu
terraform init && terraform validate   # Terraform（hashicorp/tap から入れる）
```

---

## state を置く場所について

現状はローカル state（`backend` の指定なし）。**1人で、その日のうちに
作って消す**前提なので、これで足りている。

複数人で触る、または残す運用にするなら S3 + DynamoDB のロックに移す。
その場合、state には12桁のアカウントID・Function URL・ARN が平文で入るので、
バケットの暗号化とパブリックアクセスブロックが前提になる。
ローカル state は `.gitignore` 済み。
