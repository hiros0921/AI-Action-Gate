# AI Action Gate

AIエージェントが危険な操作を実行する前に、**いったん止めて人間が承認する**仕組み。

要求を受けて、PIIを検出し、リスクを 0〜100 で採点し、**2つの閾値で三分岐**する。
LOW は自動承認、MEDIUM と HIGH は承認待ちキューへ回る。

Rust（判定エンジンとAPI）/ Svelte（承認ダッシュボード）/ AWS（実行基盤）。

> **この README は、なぜそう作ったかを書いたものです。**
> 段階ごとに追記していきます（現在：第3段階まで）。

---

## 動かし方

必要なものは **Rust 1.97 以上**だけ。第3段階ではDBも外部サービスも要りません。

```bash
cargo test                      # 78件。サーバもDBも立てずに全部通る
cargo run -p gate-api           # http://127.0.0.1:8090
```

別の端末から、疑似エージェントで投げます。

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

### 判定は I/O から独立しています

`gate-core` の依存に `tokio` も `axum` も `aws-sdk` もありません。混ざったらコンパイルが通りません。

- 判定の試験にサーバもDBも要りません（`cargo test` だけで78件）
- 判定が純粋関数なので、**同じ入力なら必ず同じ点**になります。
  ここに時刻が1つ入った瞬間、「過去の要求を新しい閾値で再判定する」が嘘になります

### 閾値シミュレーションは、本文を読み直しません

スコアの**内訳（要素・素点・重み）を保存**しているので、閾値を変えるのは保存済みの点を
振り分け直すだけ、重みを変えるのも素点に掛け直すだけです。

つまり**平文を消したあとでもシミュレーションできます**。個人情報を持ち続けなくて済みます。

そして「何件動くか」だけでなく、**動いたもののうち PII を含むものが何件か**を必ず出します。
数だけ見て緩めると、緩めてはいけないものが混ざります。

### リスクの重みと閾値は、まだ決まっていません

いま入っているのは `Policy::provisional()`（`version: "provisional-0"`）だけです。
**動かすために置いてあるだけで、根拠はありません。**

第4段階で案を複数出し、サンプル要求がどう三分岐するかを実測してから採用します。
`/api/policy` に `"adopted": false` が出るので、画面からも区別できます。

---

## 構成

```
crates/
  gate-core/   判定。PII検出・スコアリング・マスキング。I/Oを持たない
  gate-api/    axum の HTTP API。受け取って渡すだけ
  gate-agent/  疑似エージェント CLI（本物のAIエージェントは作らない）
```

## 進捗

- [x] 第2段階：判定コア（I/Oなし）
- [x] 第3段階：Rust API（インメモリ）
- [ ] 第4段階：スコアリング基準の選定 ← 次
- [ ] 第5段階：Svelte ダッシュボード
- [ ] 第6段階：DynamoDB 永続化と監査ログ
- [ ] 第7段階：AWS へのデプロイ
- [ ] 第8段階：通し確認と README 仕上げ

サンプルに出てくる氏名・電話番号・生年月日・企業名は**すべて架空**です。
