// Lambda 本体と、それを外から叩けるようにする設定。
//
// ビルドは Terraform の仕事ではない。先にこれを実行しておく:
//   cargo lambda build --release --arm64 -p gate-api

// cargo lambda が作る実行ファイルを zip に詰める。
// Lambda のカスタムランタイムは、zip の中の "bootstrap" を探して起動する。
data "archive_file" "gate_api" {
  type        = "zip"
  source_file = var.lambda_bootstrap_path
  output_path = "${path.module}/.build/gate-api.zip"
}

// 【重要】ロググループは Lambda より先に作る。
//
// Lambda は初回実行時に /aws/lambda/<関数名> を自分で作る。そのとき
// 保持期間は「無期限」になる。あとから put-retention-policy で直せるが、
// 直し忘れると静かに溜まり続ける（手動手順で実際に注意書きを入れていた箇所）。
//
// Terraform で先に作っておけば、保持期間つきで存在する状態から始まる。
// 逆に Lambda が先に作ってしまうと、Terraform 側が
// 「ResourceAlreadyExistsException」で失敗する。だから依存の向きが要る。
resource "aws_cloudwatch_log_group" "gate_api" {
  name              = "/aws/lambda/${var.function_name}"
  retention_in_days = var.log_retention_days
}

resource "aws_lambda_function" "gate_api" {
  function_name = var.function_name
  role          = aws_iam_role.gate_api.arn

  filename         = data.archive_file.gate_api.output_path
  source_code_hash = data.archive_file.gate_api.output_base64sha256

  // Rust のカスタムランタイム。handler は使われないが指定は要る。
  runtime = "provided.al2023"
  handler = "bootstrap"

  // arm64（Graviton）。x86 より約20%安く、Rust では移植の手間がない。
  architectures = ["arm64"]

  memory_size = 128
  timeout     = 10

  environment {
    variables = {
      GATE_STORE   = "dynamodb"
      GATE_POLL_MS = "3000"
      RUST_LOG     = "info"
    }
  }

  // ロググループを先に作らせる。理由は上のコメント。
  depends_on = [
    aws_cloudwatch_log_group.gate_api,
    aws_iam_role_policy.gate_api,
  ]
}

// ⚠️ この URL は、知っている人なら誰でも叩ける。
//
// authorization_type = "NONE" は意図した設定。このシステムは認証を
// 実装していない（README「認証は、意図的に実装していません」）。
// 実害を小さくしている条件は3つ:
//   - 投入するのは架空のデータだけ
//   - 予算アラートが先に入っている（budget.tf）
//   - その日のうちに terraform destroy する
//
// IAM 認証に変える場合は "AWS_IAM"。ただし curl も画面も SigV4 で
// 署名する必要が出るので、README の「curl でそのまま叩ける」形は崩れる。
resource "aws_lambda_function_url" "gate_api" {
  function_name      = aws_lambda_function.gate_api.function_name
  authorization_type = "NONE"
}

// 🔥 ここが実測で踏んだ箇所。
//
// authorization_type = "NONE" だけでは 403 が返る。
// リソースベースポリシー（関数に付く許可）が別に要る。
// 下の2つを両方入れて、初めて 200 になった。

// ① Function URL 経由の呼び出しを許可する
//
// 【実測 2026-09-04】これは重複だった。
// aws_lambda_function_url を authorization_type = "NONE" で作ると、
// プロバイダが "FunctionURLAllowPublicAccess" という同内容の statement を
// 自動で追加する。apply 後に get-policy で確認したら、①と自動追加分の
// 2つが並んでいた（infra/evidence/terraform-run.txt）。
//
// 害は無い（同じ Allow が2つあるだけ）ので、手動手順との対応が分かるよう
// 残してある。消すなら、この resource を削除して apply すればよい。
// 自動追加分は aws_lambda_function_url が管理しているので、そちらは残る。
resource "aws_lambda_permission" "function_url_invoke" {
  statement_id  = "FunctionUrlInvoke"
  action        = "lambda:InvokeFunctionUrl"
  function_name = aws_lambda_function.gate_api.function_name
  principal     = "*"

  function_url_auth_type = "NONE"
}

// ② 関数呼び出しそのものを許可する。ただし Function URL 経由に限定する。
//
// 手動手順（DEPLOY.md）ではこう書いていた:
//   aws lambda add-permission --action lambda:InvokeFunction --principal '*' \
//     --invoked-via-function-url
//
// 【実測 2026-09-03】これは aws_lambda_permission では書けない。
//
// 最初は function_url_auth_type = "NONE" を InvokeFunction に付けて書いた。
// apply で AWS に拒否された:
//   InvalidParameterValueException:
//   FunctionUrlAuthType is only supported for lambda:InvokeFunctionUrl action
//
// CLI の --invoked-via-function-url は、API の InvokedViaFunctionUrl に対応する
// 別のパラメータで、プロバイダ 5.100.0 の aws_lambda_permission には
// それに相当する属性が存在しない（tofu providers schema -json で確認。
// 持っているのは function_url_auth_type だけ）。
//
// 提供元に無いので、AWS CLI を Terraform から呼ぶ。
// 見た目は劣るが、「何をしているか」は手動手順と1文字も変わらない。
//
// 【重要】これが無いと lambda:InvokeFunction を誰にでも開くか、
// あるいは 403 のままかの二択になる。条件を付けることで、許可の範囲が
// 「URL 経由で来たものだけ」に閉じる。
//
// action の名前も紛らわしい。InvokeFunctionUrl と InvokeFunction は別物で、
// 片方だけ入れてもエラーにならず 403 が返るだけなので、原因が分かりにくい。
resource "terraform_data" "function_url_invoke_function" {
  // 変更されたら作り直す値。destroy 時の provisioner は self.input しか
  // 参照できないので、必要な値はすべてここに入れる。
  input = {
    function_name = aws_lambda_function.gate_api.function_name
    statement_id  = "FunctionUrlInvokeFunction"
    region        = var.region
  }

  // 作成。先に同名の statement を消してから足す。
  // 前回の apply が途中で落ちて権限だけ残っている場合、add-permission は
  // ResourceConflictException で止まる。消してから足せば、何度走っても同じ結果になる。
  provisioner "local-exec" {
    command = <<-EOT
      aws lambda remove-permission \
        --function-name "${self.input.function_name}" \
        --statement-id "${self.input.statement_id}" \
        --region "${self.input.region}" 2>/dev/null || true
      aws lambda add-permission \
        --function-name "${self.input.function_name}" \
        --statement-id "${self.input.statement_id}" \
        --action lambda:InvokeFunction \
        --principal '*' \
        --invoked-via-function-url \
        --region "${self.input.region}"
    EOT
  }

  // 削除。関数ごと消えるなら不要だが、この resource だけを消す場合に備える。
  // 関数が先に消えていると ResourceNotFoundException になるので、失敗は無視する。
  provisioner "local-exec" {
    when    = destroy
    command = <<-EOT
      aws lambda remove-permission \
        --function-name "${self.input.function_name}" \
        --statement-id "${self.input.statement_id}" \
        --region "${self.input.region}" 2>/dev/null || true
    EOT
  }

  // URL が先に存在している必要がある。順序を宣言する。
  depends_on = [aws_lambda_function_url.gate_api]
}
