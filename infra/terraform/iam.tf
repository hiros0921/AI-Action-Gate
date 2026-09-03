// このプロジェクトの中心。監査ログを消せなくする。
//
// 【重要】Deny は Allow より強い。あとから広い Allow を足しても抜けない。
// 「アプリのコードに削除処理を書かない」だけでは、書けば消せる状態のまま。
// 権限の側で塞ぐと、コードを書き換えても消せない。
//
// 効いていることは infra/evidence/iam-simulate-audit.json で証明済み
// （IAM Policy Simulator の出力。実際に削除を試さずに確認できる）。

data "aws_iam_policy_document" "lambda_trust" {
  statement {
    effect  = "Allow"
    actions = ["sts:AssumeRole"]

    principals {
      type        = "Service"
      identifiers = ["lambda.amazonaws.com"]
    }
  }
}

resource "aws_iam_role" "gate_api" {
  name               = "gate-api-role"
  assume_role_policy = data.aws_iam_policy_document.lambda_trust.json
}

data "aws_iam_policy_document" "gate_api" {

  // ── 監査ログ：追記だけ許す ──
  statement {
    sid    = "AuditLogsAppendOnly"
    effect = "Deny"

    actions = [
      "dynamodb:UpdateItem",
      "dynamodb:DeleteItem",

      // 【重要】BatchWriteItem を落とすのが要点。
      // DeleteItem だけを拒否しても、BatchWriteItem の中に
      // DeleteRequest を入れれば消せてしまう。ここが抜けている
      // 「追記専用」設計をよく見かける。
      "dynamodb:BatchWriteItem",

      // テーブルごと消せば、中身も消える。
      "dynamodb:DeleteTable",

      // TTL を付けられると、期限で自動的に消える。
      // 「消す」ではなく「消えるようにする」経路を塞ぐ。
      "dynamodb:UpdateTimeToLive",
    ]

    resources = [aws_dynamodb_table.audit_logs.arn]
  }

  statement {
    sid    = "AuditLogsWriteAndRead"
    effect = "Allow"

    actions = [
      "dynamodb:PutItem", // 追記はできる
      "dynamodb:Query",
      "dynamodb:Scan",
      "dynamodb:GetItem",
    ]

    resources = [aws_dynamodb_table.audit_logs.arn]
  }

  // ── 判定結果と平文：ふつうに読み書きする ──
  statement {
    sid    = "RequestsAndPayloads"
    effect = "Allow"

    actions = [
      "dynamodb:PutItem",
      "dynamodb:GetItem",
      "dynamodb:UpdateItem",
      "dynamodb:Query",
      "dynamodb:Scan",
    ]

    resources = [
      aws_dynamodb_table.action_requests.arn,

      // 索引は別の ARN になる。テーブルの ARN だけでは
      // pending_index を Query できず、実行時に AccessDenied になる。
      "${aws_dynamodb_table.action_requests.arn}/index/*",

      aws_dynamodb_table.request_payloads.arn,
    ]
  }

  // ── ログ ──
  statement {
    sid    = "Logs"
    effect = "Allow"

    actions = [
      "logs:CreateLogGroup",
      "logs:CreateLogStream",
      "logs:PutLogEvents",
    ]

    resources = [
      aws_cloudwatch_log_group.gate_api.arn,
      "${aws_cloudwatch_log_group.gate_api.arn}:*",
    ]
  }
}

resource "aws_iam_role_policy" "gate_api" {
  name   = "gate-api-policy"
  role   = aws_iam_role.gate_api.id
  policy = data.aws_iam_policy_document.gate_api.json
}
