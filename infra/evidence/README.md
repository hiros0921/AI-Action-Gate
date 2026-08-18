# 証跡

第7段階（AWS デプロイ）で保存するもの。**手順は `../DEPLOY.md`。**

| ファイル | 中身 | なぜ残すか |
|---|---|---|
| `iam-simulate-audit.json` | IAM Policy Simulator の判定結果 | **監査ログが消せないことの証明。** 実際に消しに行かずに確かめられる |
| `no-delete-in-code.txt` | コードに削除の呼び出しが無いこと | IAM とアプリの両側を見せるため |
| `aws-run.txt` | AWS 上での health / policy / queue / audit の出力 | 本当に動いたことの記録 |
| `*.png` | ダッシュボードのスクリーンショット | 面談で見せるもの |
| `teardown.txt` | 削除後の確認出力 | **消し忘れが無いことの記録。** この手のいちばんよくある事故 |

> **【重要】公開する前に伏せるもの（対応済み）。**
>
> | 伏せるもの | どこに写るか | どうしたか |
> |---|---|---|
> | **Function URL** | `aws-run.txt`・スクリーンショット | `aws_walkthrough.py` は URL を使わず invoke で通す。画面は `localhost` 経由で撮る |
> | **AWSアカウントID（12桁）** | `iam-simulate-audit.json`・`teardown.txt` の ARN | 保存する時点で `sed` が `<ACCOUNT_ID>` に置換する |
> | **ブラウザのブックマークバー** | スクリーンショットの上部 | 個人の閲覧内容が写るので、切り落とす |
>
> **コミットしてしまうと、あとから消しても履歴には残ります。**
> 伏せてから `git add` する、ではなく、**伏せた形でしか出力されない**ようにしてあります。
> 手で消す運用は、いつか忘れます。

`iam-simulate-audit.json` で見るところ。

```
dynamodb:DeleteItem      explicitDeny
dynamodb:UpdateItem      explicitDeny
dynamodb:BatchWriteItem  explicitDeny   ← DeleteItem だけ塞いでも、ここから抜けられる
dynamodb:PutItem         allowed
```
