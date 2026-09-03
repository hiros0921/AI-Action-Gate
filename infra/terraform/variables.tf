variable "region" {
  description = "デプロイ先リージョン。IAM ポリシーの ARN もこれに追従する。"
  type        = string
  default     = "ap-northeast-1"
}

variable "function_name" {
  description = "Lambda 関数名。ロググループ名 /aws/lambda/<name> もこれから決まる。"
  type        = string
  default     = "gate-api"
}

variable "budget_notification_email" {
  description = <<-EOT
    予算アラートの通知先。
    【重要】ここを空にしたままにしないこと。手動手順（DEPLOY.md 手順0）でも、
    予算アラートを他のどのリソースよりも先に作っている。デプロイしてから
    設定すると、その間に起きた課金事故に気づけない。
  EOT
  type        = string

  validation {
    condition     = can(regex("^[^@]+@[^@]+\\.[^@]+$", var.budget_notification_email))
    error_message = "メールアドレスの形式で指定してください。"
  }
}

variable "budget_limit_usd" {
  description = "月額の予算上限（USD）。実測の想定は月 $0.1 未満なので、$10 は十分に余裕がある。"
  type        = string
  default     = "10"
}

variable "lambda_bootstrap_path" {
  description = <<-EOT
    cargo lambda build --release --arm64 -p gate-api で作った実行ファイルの場所。
    Terraform はビルドしない。ビルドは cargo の仕事で、配置が Terraform の仕事。
  EOT
  type        = string
  default     = "../../target/lambda/gate-api/bootstrap"
}

variable "log_retention_days" {
  description = <<-EOT
    Lambda のログ保持日数。
    【重要】既定は「無期限」で、静かに溜まり続ける。手動手順でも明示していた箇所。
  EOT
  type        = number
  default     = 7
}
