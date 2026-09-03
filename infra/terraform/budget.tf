// 予算アラート。
//
// 【重要】手動手順（DEPLOY.md）では、これを他のどのリソース作成よりも
// 先に置いていた。デプロイしてから設定すると、その間に起きた事故に
// 気づけないため。
//
// Terraform はリソースの作成順を依存関係から決めるので、ファイルの位置では
// 順番を保証できない。そこで depends_on を使わず「先に作る」のではなく、
// 予算だけを先に apply できる形にしてある:
//
//   terraform apply -target=aws_budgets_budget.ai_action_gate
//   terraform apply
//
// README の手順もこの2段階になっている。

resource "aws_budgets_budget" "ai_action_gate" {
  name         = "ai-action-gate"
  budget_type  = "COST"
  limit_amount = var.budget_limit_usd
  limit_unit   = "USD"
  time_unit    = "MONTHLY"

  // 【重要】ACTUAL（実績）と FORECASTED（予測）の両方を入れる。
  // ACTUAL だけだと、使ってしまってから通知が来る。
  // FORECASTED は「月末にこの額になりそう」の時点で鳴るので、早く気づける。

  notification {
    comparison_operator        = "GREATER_THAN"
    threshold                  = 50
    threshold_type             = "PERCENTAGE"
    notification_type          = "ACTUAL"
    subscriber_email_addresses = [var.budget_notification_email]
  }

  notification {
    comparison_operator        = "GREATER_THAN"
    threshold                  = 80
    threshold_type             = "PERCENTAGE"
    notification_type          = "FORECASTED"
    subscriber_email_addresses = [var.budget_notification_email]
  }

  // 予算アラートは destroy しない。無料で、消し忘れに気づく最後の砦になる。
  //
  // 【実測 2026-09-04】prevent_destroy は「この1件を飛ばす」ではなく
  // 「destroy 全体を止める」。素の tofu destroy は次で止まる:
  //   Error: Resource instance cannot be destroyed
  // 通常の撤収は、この resource を除外して行う:
  //   tofu destroy -exclude="aws_budgets_budget.ai_action_gate"
  //
  // 予算そのものを消す場合は、ここを false にしてから:
  //   tofu destroy -target=aws_budgets_budget.ai_action_gate
  lifecycle {
    prevent_destroy = true
  }
}
