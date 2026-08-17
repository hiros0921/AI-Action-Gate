# 証跡

第7段階（AWS デプロイ）で保存するもの。**手順は `../DEPLOY.md`。**

| ファイル | 中身 | なぜ残すか |
|---|---|---|
| `iam-simulate-audit.json` | IAM Policy Simulator の判定結果 | **監査ログが消せないことの証明。** 実際に消しに行かずに確かめられる |
| `no-delete-in-code.txt` | コードに削除の呼び出しが無いこと | IAM とアプリの両側を見せるため |
| `aws-run.txt` | AWS 上での health / policy / queue / audit の出力 | 本当に動いたことの記録 |
| `*.png` | ダッシュボードのスクリーンショット | 面談で見せるもの |
| `teardown.txt` | 削除後の確認出力 | **消し忘れが無いことの記録。** この手のいちばんよくある事故 |

> **【重要】Function URL を伏せてから共有・公開してください。**
>
> 認証タイプは `NONE` です。動かしているあいだは、URL を知っていれば誰でも叩けます。
> `aws-run.txt` とスクリーンショットには URL が写ります。
> 止めたあとは無効な URL になりますが、**公開リポジトリの履歴からは消えません。**

`iam-simulate-audit.json` で見るところ。

```
dynamodb:DeleteItem      explicitDeny
dynamodb:UpdateItem      explicitDeny
dynamodb:BatchWriteItem  explicitDeny   ← DeleteItem だけ塞いでも、ここから抜けられる
dynamodb:PutItem         allowed
```
