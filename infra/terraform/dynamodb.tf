// DynamoDB 3テーブル。役割が違うので、設定も意図的に違えてある。
//
//   action_requests   判定結果。承認待ちを引く索引つき
//   request_payloads  平文。TTL で自動的に消える
//   audit_logs        監査ログ。TTL を付けない。消えては困る
//
// 課金はすべて PAY_PER_REQUEST（オンデマンド）。
// 短時間の検証で、読み書きの見積もりが立たないため。プロビジョンド
// にすると使わない容量に払うことになる。

resource "aws_dynamodb_table" "action_requests" {
  name         = "action_requests"
  billing_mode = "PAY_PER_REQUEST"
  hash_key     = "pk"

  attribute {
    name = "pk"
    type = "S"
  }

  // GSI で使う属性も、ここで宣言しないと apply が通らない。
  attribute {
    name = "pending_key"
    type = "S"
  }

  attribute {
    name = "created_at"
    type = "S"
  }

  // 承認待ちの一覧を引くための索引。
  //
  // 【重要】これが無いと Scan になる。Scan は全件読むので、
  // 件数が増えると料金と遅延がそのまま比例して増える。
  // 承認待ちは「全体のごく一部」なので、索引で引く形が正しい。
  global_secondary_index {
    name            = "pending_index"
    hash_key        = "pending_key"
    range_key       = "created_at"
    projection_type = "ALL"
  }
}

resource "aws_dynamodb_table" "request_payloads" {
  name         = "request_payloads"
  billing_mode = "PAY_PER_REQUEST"
  hash_key     = "pk"

  attribute {
    name = "pk"
    type = "S"
  }

  // 平文の本文はここにだけ入る。だから、ここにだけ TTL を付ける。
  // 判定結果（action_requests）と監査ログ（audit_logs）は残り、
  // 中身だけが期限で消える形になる。
  ttl {
    attribute_name = "expires_at"
    enabled        = true
  }
}

resource "aws_dynamodb_table" "audit_logs" {
  name         = "audit_logs"
  billing_mode = "PAY_PER_REQUEST"
  hash_key     = "pk"
  range_key    = "sk"

  attribute {
    name = "pk"
    type = "S"
  }

  attribute {
    name = "sk"
    type = "S"
  }

  // 【重要】このテーブルに ttl ブロックは書かない。意図的な不在。
  //
  // TTL を付けると、期限で自動的に消える。監査ログでそれをやると、
  // 「消せない」と言いながら時間が経てば消える、という状態になる。
  // IAM 側で UpdateTimeToLive も Deny しているので、あとから
  // 実行ロールが付け直すこともできない（iam.tf 参照）。

  // 消せないことと、壊れたときに戻せることは別の話。
  // Deny で守るのは人為的・プログラム的な削除。PITR が守るのは
  // 事故と障害。両方要る。
  point_in_time_recovery {
    enabled = true
  }
}
