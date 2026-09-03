// プロバイダとバージョンの固定。
//
// 【重要】バージョンを固定しないと、半年後に同じコードが違う結果を作る。
// IaC の目的は「同じものが再現できること」なので、ここを緩めると目的が消える。

terraform {
  required_version = ">= 1.6"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.60"
    }
    archive = {
      source  = "hashicorp/archive"
      version = "~> 2.4"
    }
  }
}

provider "aws" {
  region = var.region

  // 【実測 2026-09-03】ここに default_tags を書いて失敗した。記録として残す。
  //
  //   AccessDeniedException: not authorized to perform: budgets:TagResource
  //
  // deployer-policy.json は手動手順（AWS CLI）で必要だったぶんだけを
  // 許可しており、Tag 系のアクションが1つも入っていない。タグを付けると
  // budgets / dynamodb / lambda / iam / logs の5つで同じエラーが出る。
  //
  // 対処は2つあった:
  //   A. deployer-policy.json に Tag 系5つを足す
  //   B. タグ付けをやめる
  //
  // B を選んだ。タグを入れた理由は「消し忘れを探しやすくするため」だが、
  // Terraform では state が作成物の一覧そのものなので、目的が重複している。
  // 要らない機能のために権限を5つ広げるのは筋が悪い。
  // DEPLOY.md にも「先に広げないでください」と書いてある。
}

// アカウントIDは変数にしない。ここから取る。
//
// 【重要】手動手順（DEPLOY.md）では ACCOUNT_ID を文字列置換していたため、
// 証跡ファイルに12桁が残り、git add の前に sed で伏せる作業が必要だった。
// データソースで解決すれば、コードにもtfvarsにも12桁は現れない。
data "aws_caller_identity" "current" {}
