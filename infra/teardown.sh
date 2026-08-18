#!/usr/bin/env bash
#
# AWS 上の資源をすべて消して、消えたことを確かめる。
#
# 【重要】「消したつもりで残っていた」が、この手のいちばんよくある事故です。
# 消すコマンドを流すだけでは足りません。消えたことを確認して、記録に残します。
# 出力は infra/evidence/teardown.txt に入ります。
#
# 【重要】アクセスキーの無効化は、このスクリプトではやりません。
# キーを止めると、以降の確認コマンドが全部通らなくなるためです。
# teardown.txt を目で確かめたあと、最後に手で止めます（README / DEPLOY.md 手順7）。
#
# 実行:
#   bash infra/teardown.sh
#
set -uo pipefail   # 【重要】set -e は付けない。
                   # 「すでに無い」もこのスクリプトでは正常なので、
                   # 途中で止まらずに最後まで流して、結果を全部残す。

REGION=ap-northeast-1
OUT=infra/evidence/teardown.txt
ACCOUNT_ID=$(aws sts get-caller-identity --query Account --output text)

exec > >(sed "s/${ACCOUNT_ID}/<ACCOUNT_ID>/g" | tee "$OUT") 2>&1

echo "=== 削除 ==="
echo "日時: $(date '+%Y-%m-%d %H:%M:%S %Z')"
echo "リージョン: ${REGION}"
echo

echo "-- Lambda --"
aws lambda delete-function-url-config --function-name gate-api 2>&1 | tail -2
aws lambda delete-function --function-name gate-api 2>&1 | tail -2
echo "  削除コマンドを実行しました"

echo "-- DynamoDB --"
for t in action_requests request_payloads audit_logs; do
  aws dynamodb delete-table --table-name "$t" --query 'TableDescription.TableStatus' --output text 2>&1 | tail -1
done

echo "-- CloudWatch Logs --"
# 【重要】Lambda を消してもロググループは残る。消し忘れの定番。
aws logs delete-log-group --log-group-name /aws/lambda/gate-api 2>&1 | tail -2
echo "  削除コマンドを実行しました"

echo "-- IAM ロール --"
# ロールに付いたポリシーを先に外さないと、ロールは消せない。
aws iam delete-role-policy --role-name gate-api-role --policy-name gate-api-policy 2>&1 | tail -2
aws iam delete-role --role-name gate-api-role 2>&1 | tail -2
echo "  削除コマンドを実行しました"

echo
echo "テーブルが消えるまで待ちます（DynamoDB の削除は非同期）..."
for t in action_requests request_payloads audit_logs; do
  aws dynamodb wait table-not-exists --table-name "$t" 2>/dev/null
done
echo "待機終了"

echo
echo "=== 消えたことの確認 ==="
echo "（すべて「なし」なら、課金対象は残っていません）"
echo

check() {  # 名前, 確認コマンド
  local label="$1"; shift
  local result
  result=$("$@" 2>&1)
  if echo "$result" | grep -qi "ResourceNotFound\|NoSuchEntity\|not exist\|cannot be found"; then
    echo "  ${label}: なし"
  elif [ -z "$result" ] || [ "$result" = "None" ] || [ "$result" = "[]" ]; then
    echo "  ${label}: なし"
  else
    echo "  ${label}: !! 残っています -> ${result}"
  fi
}

check "Lambda 関数 gate-api" \
  aws lambda get-function --function-name gate-api --query 'Configuration.FunctionName' --output text
check "テーブル action_requests" \
  aws dynamodb describe-table --table-name action_requests --query 'Table.TableStatus' --output text
check "テーブル request_payloads" \
  aws dynamodb describe-table --table-name request_payloads --query 'Table.TableStatus' --output text
check "テーブル audit_logs" \
  aws dynamodb describe-table --table-name audit_logs --query 'Table.TableStatus' --output text
check "IAM ロール gate-api-role" \
  aws iam get-role --role-name gate-api-role --query 'Role.RoleName' --output text
check "ロググループ /aws/lambda/gate-api" \
  aws logs describe-log-groups --log-group-name-prefix /aws/lambda/gate-api --query 'logGroups[].logGroupName' --output text

echo
echo "-- 念のため、このリージョンに他の残りが無いか --"
echo "  Lambda 関数: $(aws lambda list-functions --query 'length(Functions)' --output text) 個"
echo "  DynamoDB テーブル: $(aws dynamodb list-tables --query 'TableNames' --output text)"

echo
echo "=== 残したもの ==="
echo "  予算アラート（\$5 / \$8）: 残す。無料で、消し忘れの保険になる"
echo "  IAM ユーザ gate-deployer: 残す。ただしアクセスキーは次の手順で無効化する"
echo
echo "=== 次にやること（手で実行） ==="
echo "  aws iam list-access-keys --user-name gate-deployer"
echo "  aws iam update-access-key --user-name gate-deployer --access-key-id <ID> --status Inactive"
echo "  ※ 無効化するとこの端末から AWS を触れなくなります。上の確認が済んでから。"
