// 【重要】アカウントIDと Function URL は output に出さない。
//
// output はコンソールに表示され、CI のログにも残る。手動手順では
// 証跡ファイルから sed でこの2つを伏せる作業を入れていた（DEPLOY.md
// 「証跡を伏せる」）。同じものを output で毎回表示させては意味がない。
//
// URL が必要なときは、その場で取る:
//   terraform output -raw function_url
// sensitive = true にしてあるので、apply の最後には表示されない。

output "function_url" {
  description = "Function URL。認証なしなので、共有・スクリーンショットに注意。"
  value       = aws_lambda_function_url.gate_api.function_url
  sensitive   = true
}

output "audit_logs_table_arn" {
  description = "IAM Policy Simulator に渡す ARN。Deny が効いていることの確認に使う。"
  value       = aws_dynamodb_table.audit_logs.arn
  sensitive   = true
}

output "role_arn" {
  description = "Lambda 実行ロール。Simulator の --policy-source-arn に渡す。"
  value       = aws_iam_role.gate_api.arn
  sensitive   = true
}

// 消し忘れの確認用。destroy 後にこれが空になることを確かめる。
output "created_table_names" {
  description = "作成した DynamoDB テーブル名。destroy 後の確認に使う。"
  value = [
    aws_dynamodb_table.action_requests.name,
    aws_dynamodb_table.request_payloads.name,
    aws_dynamodb_table.audit_logs.name,
  ]
}
