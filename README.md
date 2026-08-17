# AI Action Gate

AIエージェントが危険な操作を実行する前に、**いったん止めて人間が承認する**仕組み。

要求を受けて、PIIを検出し、リスクを 0〜100 で採点し、**2つの閾値で三分岐**する。
LOW は自動承認、MEDIUM と HIGH は承認待ちキューへ回る。

Rust（判定エンジンとAPI）/ Svelte（承認ダッシュボード）/ AWS（実行基盤）。

> **この README は、なぜそう作ったかを書いたものです。**
> 段階ごとに追記していきます（現在：第5段階まで）。

---

## 動かし方

必要なものは **Rust 1.97 以上**と、ダッシュボードを動かすなら **Node 22 以上**。
第5段階まではDBも外部サービスも要りません。

```bash
cargo test                      # 103件。サーバもDBも立てずに全部通る
cargo run -p gate-api           # http://127.0.0.1:8090（メモリに保存）
```

DynamoDB に保存する場合（第6段階）。

```bash
docker compose up -d            # DynamoDB Local（ポート 18000）
GATE_STORE=dynamodb GATE_DYNAMO_ENDPOINT=http://localhost:18000 \
  AWS_ACCESS_KEY_ID=local AWS_SECRET_ACCESS_KEY=local AWS_REGION=ap-northeast-1 \
  GATE_POLICY=policies/adopted.toml cargo run -p gate-api

# DB が要る試験は明示的に呼ぶ（cargo test に Docker を要求しないため）
GATE_DYNAMO_ENDPOINT=http://localhost:18000 AWS_ACCESS_KEY_ID=local \
  AWS_SECRET_ACCESS_KEY=local AWS_REGION=ap-northeast-1 \
  cargo test -p gate-store -- --ignored --test-threads=1
```

承認ダッシュボードは別の端末で。

```bash
cd dashboard && npm install && npm run dev    # http://localhost:5173
```

疑似エージェントから要求を投げます。

```bash
cargo run -p gate-agent -- send --scenario low      # 自動承認される
cargo run -p gate-agent -- send --scenario medium   # 承認待ちになる
cargo run -p gate-agent -- send --scenario high     # マスキング案つきで承認待ち
cargo run -p gate-agent -- send --scenario high --wait   # 承認が下りるまで待つ
cargo run -p gate-agent -- status req-33dc4d77
```

ポートは 8090。8080 は別プロジェクトが使っているためずらしてあります（`GATE_ADDR` で変更可）。

### curl で叩く

CLI だけだと API の形が見えないので、生の形を置いておきます。

**① エージェントが要求を投げる**

```bash
curl -s -X POST http://127.0.0.1:8090/api/requests \
  -H 'content-type: application/json' \
  -d '{
    "agent_id": "agent-001",
    "action": "send_external",
    "destination": "external_ai",
    "data_class": "personal_information",
    "payload": { "text": "山田太郎さん（1980年3月15日生・090-1234-5678）の検査結果について" }
  }'
```

```json
{
  "request_id": "req-33dc4d77",
  "decision": "pending",
  "risk": "HIGH",
  "score": 100,
  "detected": [
    { "kind": "phone_number", "count": 1, "lowest_confidence": "high" },
    { "kind": "birth_date",   "count": 1, "lowest_confidence": "high" },
    { "kind": "person_name",  "count": 1, "lowest_confidence": "high" }
  ],
  "masked_preview": "○○○○さん（○○○○年○月○○日生・○○○-○○○○-○○○○）の検査結果について",
  "conclusive": true,
  "raised_by_uncertainty": false
}
```

**② 承認待ちを見る**（スコアの内訳とマスキングプレビューが入っています）

```bash
curl -s http://127.0.0.1:8090/api/queue
```

**③ 承認する**（`X-Approver-Id` が無いと 401）

```bash
curl -s -X POST http://127.0.0.1:8090/api/requests/req-33dc4d77/decision \
  -H 'content-type: application/json' \
  -H 'X-Approver-Id: suwa' \
  -d '{"verdict": "approve_masked"}'      # approve / approve_masked / reject
```

**④ エージェントが結果を取りに行く**

```bash
curl -s http://127.0.0.1:8090/api/requests/req-33dc4d77
```

```json
{
  "request_id": "req-33dc4d77",
  "status": "approved_masked",
  "risk": "HIGH",
  "score": 100,
  "payload": "○○○○さん（○○○○年○月○○日生・○○○-○○○○-○○○○）の検査結果について",
  "reviewer": "suwa",
  "decided_at": "2026-08-17T14:41:14+00:00"
}
```

**⑤ 閾値シミュレーション**

```bash
curl -s -X POST http://127.0.0.1:8090/api/simulate \
  -H 'content-type: application/json' -d '{"medium_at": 80, "high_at": 99}'
```

```
現在   LOW 1 / MEDIUM 1 / HIGH 1   人手に回る割合 67%
変更後 LOW 2 / MEDIUM 0 / HIGH 1   人手に回る割合 33%
→ 自動承認が 1件 増えます。うち PII を含むものが 1件
```

**⑥ 監査ログ**

```bash
curl -s http://127.0.0.1:8090/api/audit
```

---

## 設計判断

### 認証は、意図的に実装していません

承認者は HTTPヘッダ `X-Approver-Id` で名乗るだけです。**誰でも名乗れます。**
プロトタイプとして意図した割り切りで、隠していません。

そのうえで、**差し替えられる形**にしてあります。

- ドメイン層（`gate_core::review`）は `approver_id` を**文字列として受け取るだけ**で、
  それがヘッダから来たのか JWT から来たのかを知りません。
- 取り出しているのは `gate-api/src/approver.rs` の1ファイルだけ。
  Cognito でも JWT でも社内SSOでも、**ここを書き換えれば済みます**。

認証を作らないことと、認証を後から入れられないことは別です。

ただし「誰が承認したか」が空の記録は作れません。`X-Approver-Id` が無ければ **401** で、
承認そのものが通りません。自動承認（LOW）も `system` として監査ログに残します。
空文字で埋めると、あとから「人が承認したのか、自動だったのか」が区別できなくなるためです。

### マスキングは桁数を保持します

`090-1234-5678` → `○○○-○○○○-○○○○`、`1980年3月15日` → `○○○○年○月○○日`。

規則を1つにするためです。「電話番号は桁を残し、日付は潰す」だと規則が2つになり、
**なぜこれはこう伏せたのかを毎回説明することになります。**
区切り記号と単位（年月日）は残し、中身だけを伏せます。

### 「検出できなかった」と「無かった」を型で分けています

```rust
pub enum Scan {
    Scanned { findings: Vec<Finding> },   // 走査した。空なら本当に0件
    NotScanned { reason: SkipReason },    // 走査していない。0件とは言えない
}
```

`Vec<Finding>` ひとつで表すと、**両方とも空配列**になります。そうなると、
いちばん危ない「読めなかった本文」が、いちばん安全な「PIIの無い本文」と
同じ扱いで自動承認されます。

走査していない要求は、点が低くても LOW にしません（`unscanned_floor`）。
閾値シミュレーションでどれだけ緩めても、自動承認には落ちません。

### 氏名の検出には確信度が付きます

日本語の氏名は機械的には確定できません。「田中」は姓でもあり地名でもあります。
そこで、**何を根拠に見つけたか**を残し、その強さを確信度にしています。

| 確信度 | 根拠 | 例 |
|---|---|---|
| High | 敬称、またはラベル | `山田太郎さん` / `氏名：山田太郎` |
| Medium | 姓の辞書 ＋ 直後が名らしい | `山田太郎の検査結果` |
| Low | 形が似ているだけ | `タナカ・タロウ` |

Low を捨てないのは、捨てた瞬間に「無かった」ことになるからです。
承認画面には出し、承認者が「これは人名ではない」と1件だけ外せる形にしてあります。

**このシステムを入れれば個人情報が漏れない、とは言えません。** 検出漏れは原理的に残ります。

### リアルタイム更新に、なぜポーリングを選んだか

承認待ちが増えたら、リロードせずに画面へ出ます。**3秒ごとのポーリング**です。
SSE も WebSocket も使っていません。**どちらも検討したうえで落としました。**

**SSE を落とした理由。** API Gateway（HTTP API / REST API）は**レスポンスのストリーミングに
対応していません**。SSE を通すには Lambda Function URL（`RESPONSE_STREAM`）に寄せることになり、
API Gateway を使う構成から外れます。**ローカルでは動くが本番では動かない**実装を抱えることになります。

**WebSocket を落とした理由。** API Gateway には WebSocket API があるので、これは「使えない」わけでは
ありません。落としたのは**接続管理のコスト**です。Lambda は接続を保持できないので、
`connection_id` を DynamoDB に持ち、切断時に消し、送信のたびに引く必要があります。
テーブルが1つ増え、その掃除の面倒も増えます。**承認待ちが1日に数件という前提には重すぎます。**

**ポーリングで足りる理由。** 承認の発生は不定期かつ低頻度で、3秒の遅れが承認業務を損ないません。
「速いから SSE」ではなく、**遅くて困る場面が無いからポーリングで足りる**、という判断です。

そのうえで、無駄が出ないようにしてあります。

- **ETag で 304 を返します。** 変わっていなければ本文を作らず、JSON 化もマスキングもしません。
  画面側も再描画しないので、開いたまま放置しても点滅しません。
- **間隔はサーバが持ちます**（`GATE_POLL_MS`、既定3秒）。画面に焼き込んでいません。
  304 でも Lambda の呼び出し回数は数えられるので、**間隔がそのまま費用に効きます**。
  第7段階で月額を測ったあと、ここだけを動かして調整できます。

### 平文をいつ消すか — 3つのテーブルで、消し方を分けています

| テーブル | 中身 | 消える？ |
|---|---|---|
| `action_requests` | 判定結果・内訳・そのときの閾値。**平文なし** | 消しません |
| `request_payloads` | 本文（平文）だけ | **消えます**（下記） |
| `audit_logs` | 監査ログ | **消せません**（更新も削除も呼びません） |

**平文と判定結果を同じアイテムに置いていません。** DynamoDB の TTL はアイテム単位で丸ごと消すので、
同居させると内訳も閾値も一緒に消えます。それでは「平文を消したあとでも閾値シミュレーションができる」
という利点が失われます。

**平文の保持期間は、正確にはこうです。**

> **承認完了から24時間で削除対象になります。実際の削除は最大48時間後になることがあるため、
> アプリケーション側で期限切れの平文を返しません。**

DynamoDB の TTL は「期限を過ぎてから通常48時間以内」に削除される仕組みで、
削除されるまでのあいだ Query や Scan の結果に出続けます。
`PayloadStore::get_payload` が読むたびに期限を確かめているのはそのためです。
これが無いと、期限を過ぎた平文が読めてしまいます。

**承認待ちのまま放置されたものは、7日で「期限切れ」として閉じます。**
TTL は判断が下りてから動き出すので、判断が下りない要求には時計が動きません。
閉じた記録は監査ログにも残します。**判断しなかったことも記録です。**
そのため `Verdict` に `Expired` があり、人の「拒否」とは別に扱っています。
どちらも実行はさせませんが、**拒否は人が見て決めたこと、期限切れは誰も見なかったこと**です。

### 判定は I/O から独立しています

`gate-core` の依存に `tokio` も `axum` も `aws-sdk` もありません。混ざったらコンパイルが通りません。

- 判定の試験にサーバもDBも要りません（`cargo test` だけで103件）
- 判定が純粋関数なので、**同じ入力なら必ず同じ点**になります。
  ここに時刻が1つ入った瞬間、「過去の要求を新しい閾値で再判定する」が嘘になります

### 閾値シミュレーションは、本文を読み直しません

スコアの**内訳（要素・素点・重み）を保存**しているので、閾値を変えるのは保存済みの点を
振り分け直すだけ、重みを変えるのも素点に掛け直すだけです。

つまり**平文を消したあとでもシミュレーションできます**。個人情報を持ち続けなくて済みます。

そして「何件動くか」だけでなく、**動いたもののうち PII を含むものが何件か**を必ず出します。
数だけ見て緩めると、緩めてはいけないものが混ざります。

### リスクの重みと閾値は、実測してから決めました

**採用値: 重み 操作20 / 送信先40 / 区分20 / PII20、閾値 MEDIUM 20 / HIGH 55。**

29件のサンプル要求を用意し、重み案4つ・閾値案5つをそれぞれ当てて分布を測ってから選びました。
**重みと閾値は同時に変えていません**。同時に動かすと、差がどちらから来たのか分からなくなるためです。

```bash
cargo run -p gate-lab -- samples      # 比較に使った材料
cargo run -p gate-lab -- weights      # 重み案の比較（閾値は固定）
cargo run -p gate-lab -- thresholds   # 閾値案の比較（重みは固定）
```

選定の理由はコード（`policy.rs` の `Policy::adopted`）に残してあります。要点は3つです。

- **送信先を最も重くした。** 社内の削除はバックアップで戻せる可能性がありますが、
  外部AIに渡った情報は取り返せません。同じ「事故」でも後戻りできるかが違います。
- **MEDIUM を 20 にした。** 25 にすると「データ区分の申告なし」（22点）が自動承認され、
  **区分を申告しないほうが通りやすくなります**。エージェント側から見ると
  「`data_class` を書かなければ通る」抜け道です。悪意がなくても実装の手抜きで発生します。
- **HIGH を 55 にした。** 人手の総数は変わらず（23件のまま）、HIGH が3件から6件に増えるだけです。
  これがないと承認者の選択肢は「拒否」か「そのまま承認」の二択になります。
  伏せて通せると分かっていれば、拒否せずに済む場面が増えます。

材料づくりでも1つ学びがありました。**推奨された 13〜25 の帯に、サンプルが1件もありませんでした。**
そのままでは閾値案3つの結果が完全に一致し、比較になりません。材料に無い帯では案の差が測れないので、
その帯の要求を足してから測り直しています。

---

## AWS の想定月額（第7段階のデプロイ前）

### 前提

**前提なしの金額は意味がないので、先に書きます。**

| 前提 | 値 | 備考 |
|---|---|---|
| 実行要求 | **30件/日** | デモと動作確認。実運用ではない |
| 承認者 | **1人** | |
| ダッシュボードを開いている時間 | **1日8時間 × 月20日** | **ここが効きます** |
| ポーリング間隔 | **3秒**（`GATE_POLL_MS`） | |
| リージョン | ap-northeast-1（東京） | |
| 保存されている要求 | 50件 | 読み取り費用がここに比例します |

### 費用は「要求の件数」では決まりません

3秒間隔で画面を開いたままにすると、**1時間で1,200リクエスト**。8時間で9,600、月20日で
**192,000リクエスト**です。承認待ちが1日3件でも、この数字は変わりません。

**費用を決めるのは「画面を開いている時間 × 保存件数」です。**

| 内訳 | 月あたりのリクエスト |
|---|---|
| ダッシュボードのポーリング（承認待ちタブ） | 192,000 |
| 疑似エージェント（要求30件/日 ＋ 結果の取得） | 約 5,000 |
| **合計** | **約 197,000** |
| （参考）履歴・監査タブを開いたままにすると | 約 581,000 |

最後の行は、いまの画面が承認待ち以外のタブでは3つのAPIを叩くためです。

### サービスごと

| サービス | 数量 | 12ヶ月以内 | 13ヶ月目以降 |
|---|---|---|---|
| **Lambda** | 197,000回 × 30ms × 128MB ＝ 約740 GB秒 | **$0**（常時無料: 100万回・40万GB秒） | $0 |
| **API Gateway**（HTTP API） | 197,000リクエスト | **$0**（**12ヶ月無料**: 100万/月） | **約 $0.24** |
| **DynamoDB** 書き込み | 約5,400 WRU | 約 $0.01 | 約 $0.01 |
| **DynamoDB** 読み取り | 約 605万 RRU | **約 $1.72** | 約 $1.72 |
| **DynamoDB** 保存 | 1MB未満 | $0（25GBまで常時無料） | $0 |
| **CloudWatch Logs** | 約 59MB | $0（5GBまで常時無料） | $0 |
| **データ転送** | 約 0.4GB | $0（100GB/月まで無料） | $0 |
| **合計** | | **約 $1.8/月** | **約 $2.0/月** |

**無料枠は2種類あります。** Lambda と DynamoDB と CloudWatch は**常時無料**の枠ですが、
**API Gateway は12ヶ月無料**です。1年後に課金が始まります（上表の右列）。

DynamoDB のオンデマンドには注意があります。**無料枠の「25 WCU / 25 RCU」はプロビジョンドモード専用**で、
オンデマンドには適用されません。1リクエスト目から課金されます（金額は上表のとおり小さい）。

### 主役は DynamoDB の読み取りです

ポーリング1回ごとに、承認待ちを取るために `action_requests` を **Scan** し、
件数ぶん `request_payloads` を **GetItem** しています。**保存件数に比例して増えます。**

| 保存件数 | 1回あたり | 月額（読み取り） |
|---|---|---|
| 20件 | 約13 RRU | 約 $0.7 |
| 50件 | 約32 RRU | **約 $1.7** |
| 100件 | 約63 RRU | 約 $3.4 |
| 500件 | 約313 RRU | 約 $17 |

ETag で 304 を返す道でも、**変わっていないことを確かめるために同じ Scan をしています。**
本文の生成は止まりますが、読み取り費用は止まりません。

### 最悪のケース

履歴タブを開いたまま、保存が100件まで増えた場合: 読み取り 約$10 ＋ API Gateway $0.71 ＝ **約 $11/月**。

### CloudWatch Logs の保持期間

**既定は「無期限」です。** 設定しないとログが溜まり続け、静かに課金されます。
**7日**を明示的に設定します（デプロイ手順に含めます）。

```bash
aws logs put-retention-policy --log-group-name /aws/lambda/gate-api --retention-in-days 7
```

---

## 止め方（デプロイしたあと）

**このプロジェクトは実績づくりで、運用しません。** 動作確認とスクリーンショットが済んだら止めます。

消し忘れると課金が続くものを、順に消します。

```bash
# ① API Gateway（12ヶ月を過ぎるとリクエスト課金）
aws apigatewayv2 delete-api --api-id <API_ID>

# ② Lambda 関数
aws lambda delete-function --function-name gate-api

# ③ DynamoDB の3テーブル（保存が課金対象。25GBまでは無料だが、消し忘れない）
aws dynamodb delete-table --table-name action_requests
aws dynamodb delete-table --table-name request_payloads
aws dynamodb delete-table --table-name audit_logs

# ④ CloudWatch Logs のロググループ
#    【重要】Lambda を消してもロググループは残ります。ここが消し忘れの定番。
aws logs delete-log-group --log-group-name /aws/lambda/gate-api

# ⑤ IAM ロールとポリシー（課金はされないが、残すと権限が残る）
aws iam delete-role-policy --role-name gate-api-role --policy-name gate-api-policy
aws iam delete-role --role-name gate-api-role

# ⑥ 消え残りがないか確認
aws dynamodb list-tables
aws logs describe-log-groups --log-group-name-prefix /aws/lambda/gate
aws apigatewayv2 get-apis --query 'Items[].Name'
```

**AWS Budgets のアラートは残して構いません**（無料）。消し忘れに気づく最後の砦になります。

---

## 構成

```
crates/
  gate-core/   判定。PII検出・スコアリング・マスキング。I/Oを持たない
  gate-api/    axum の HTTP API。受け取って渡すだけ
  gate-agent/  疑似エージェント CLI（本物のAIエージェントは作らない）
  gate-lab/    案を実測で比べる道具（重み・閾値の選定に使った）
  gate-store/  置き場。メモリ版と DynamoDB 版（平文は別テーブル）
dashboard/     承認ダッシュボード（Vite + Svelte）
policies/      採用された設定
```

ダッシュボードは **SvelteKit ではなく Vite + Svelte** です。
ルーティングもSSRも要らない1画面なので、仕様書10章「大規模なフロントエンドフレームワークを
追加しない」に沿ってこちらにしました。

## 進捗

- [x] 第2段階：判定コア（I/Oなし）
- [x] 第3段階：Rust API（インメモリ）
- [x] 第4段階：スコアリング基準の選定（実測して採用）
- [x] 第5段階：Svelte ダッシュボード
- [x] 第6段階：DynamoDB 永続化と監査ログ
- [ ] 第7段階：AWS へのデプロイ
- [ ] 第8段階：通し確認と README 仕上げ

サンプルに出てくる氏名・電話番号・生年月日・企業名は**すべて架空**です。
