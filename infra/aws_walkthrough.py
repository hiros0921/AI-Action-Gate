#!/usr/bin/env python3
"""AWS 上の gate-api を、LOW / MEDIUM / HIGH の3経路で通す。

【なぜ HTTP ではなく aws lambda invoke なのか】
URL に依存せずに確認できるようにするため。Function URL は削除しても
関数は残るし、逆に URL の設定ミスで確認が止まることもない。
確かめたいのは「AWS 上で動くこと」であって「URL が生えていること」ではない。
（実際、構築中に Function URL が 403 を返し続けた期間があった。
  そのときも、この方法で関数側の正常動作は確認できていた。）

実行:
    python3 infra/aws_walkthrough.py

infra/evidence/aws-run.txt に記録を残す。
アカウントIDや URL は出力に含まれない（証跡をそのまま公開できる）。
"""

import json
import pathlib
import subprocess
import sys
import tempfile

FUNCTION = "gate-api"
EVIDENCE = pathlib.Path(__file__).resolve().parent / "evidence" / "aws-run.txt"

lines: list[str] = []


def out(text: str = "") -> None:
    print(text)
    lines.append(text)


def invoke(method: str, path: str, body: dict | None = None, approver: str | None = None) -> dict:
    """Function URL のイベント形式（payload format 2.0）で関数を呼ぶ。"""
    headers = {"host": "invoke.local", "content-type": "application/json"}
    if approver:
        # 【重要】承認者IDはヘッダで渡す。ドメイン層は「誰から来たか」を知らない。
        headers["x-approver-id"] = approver

    event = {
        "version": "2.0",
        "routeKey": "$default",
        "rawPath": path,
        "rawQueryString": "",
        "headers": headers,
        "requestContext": {
            "http": {
                "method": method,
                "path": path,
                "protocol": "HTTP/1.1",
                "sourceIp": "127.0.0.1",
                "userAgent": "walkthrough",
            },
            "accountId": "anonymous",
            "apiId": "invoke",
            "domainName": "invoke.local",
            "domainPrefix": "invoke",
            "requestId": "walkthrough",
            "routeKey": "$default",
            "stage": "$default",
            "time": "-",
            "timeEpoch": 0,
        },
        "isBase64Encoded": False,
    }
    if body is not None:
        event["body"] = json.dumps(body, ensure_ascii=False)

    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False, encoding="utf-8") as ev:
        json.dump(event, ev, ensure_ascii=False)
        event_path = ev.name
    response_path = tempfile.NamedTemporaryFile(suffix=".json", delete=False).name

    result = subprocess.run(
        [
            "aws", "lambda", "invoke",
            "--function-name", FUNCTION,
            "--cli-binary-format", "raw-in-base64-out",
            "--payload", f"file://{event_path}",
            response_path,
            "--query", "StatusCode",
            "--output", "text",
        ],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        out(f"  !! aws lambda invoke が失敗: {result.stderr.strip()}")
        sys.exit(1)

    raw = pathlib.Path(response_path).read_text(encoding="utf-8")
    envelope = json.loads(raw)
    if "statusCode" not in envelope:
        # 関数が落ちた場合はここに errorMessage が入る。
        out(f"  !! 関数がエラーを返しました: {raw[:400]}")
        sys.exit(1)

    status = envelope["statusCode"]
    payload = json.loads(envelope["body"]) if envelope.get("body") else {}
    out(f"  {method} {path} -> HTTP {status}")
    return {"status": status, "body": payload}


def show(label: str, data: dict, keys: list[str]) -> None:
    out(f"  {label}")
    for key in keys:
        if key in data:
            value = data[key]
            rendered = json.dumps(value, ensure_ascii=False) if isinstance(value, (dict, list)) else value
            out(f"    {key}: {rendered}")


LOW = {
    "agent_id": "agent-001",
    "action": "read",
    "destination": "internal",
    "data_class": "public",
    "payload": {"text": "在庫一覧を確認します。対象は倉庫Aの全SKUです。"},
}
MEDIUM = {
    "agent_id": "agent-001",
    "action": "write",
    "destination": "external",
    "data_class": "internal",
    "payload": {"text": "取引先へ発注書を送信します。連絡先は order@example.com です。"},
}
HIGH = {
    "agent_id": "agent-001",
    "action": "send_external",
    "destination": "external_ai",
    "data_class": "personal_information",
    "payload": {"text": "山田太郎さん（1980年3月15日生・090-1234-5678）の検査結果について要約してください。"},
}

out("== AWS 上の gate-api 通し確認 ==")
out("Lambda（ap-northeast-1）+ DynamoDB。URL に依存しないよう直接 invoke で確認する。")
out()

out("[0] 稼働確認")
health = invoke("GET", "/api/health")
show("health", health["body"], ["status", "store", "policy"])
out()

out("[1] LOW: 社内の読み取り・個人情報なし → 自動で通す")
low = invoke("POST", "/api/requests", LOW)
show("結果", low["body"], ["decision", "risk", "score", "detected", "conclusive"])
out()

out("[2] MEDIUM: 社外への書き込み・メールアドレスあり → 承認待ちで止める")
medium = invoke("POST", "/api/requests", MEDIUM)
show("結果", medium["body"], ["decision", "risk", "score", "detected", "raised_by_uncertainty"])
medium_id = medium["body"]["request_id"]
out()

out("[3] HIGH: 外部AIへ個人情報 → 承認待ち＋マスキング案")
high = invoke("POST", "/api/requests", HIGH)
show("結果", high["body"], ["decision", "risk", "score", "detected", "masked_preview"])
high_id = high["body"]["request_id"]
out()

out("[4] 承認待ちの一覧（sparse GSI で引く。件数に依存しない）")
queue = invoke("GET", "/api/queue")
items = queue["body"]
out(f"    待ち件数: {len(items)}")
for item in items:
    out(f"    - {item['request_id'][:8]}… risk={item['risk']} score={item['score']}")
out()

out("[5] MEDIUM を人が承認する（承認者IDはヘッダで渡す）")
decided = invoke("POST", f"/api/requests/{medium_id}/decision", {"verdict": "approve"}, approver="suwa")
show("結果", decided["body"], ["verdict", "reviewer", "at"])
out()

out("[6] 同じ要求をもう一度承認しようとする（条件式で弾かれるはず）")
again = invoke("POST", f"/api/requests/{medium_id}/decision", {"verdict": "approve"}, approver="suwa")
out(f"    二重承認の応答: HTTP {again['status']} {json.dumps(again['body'], ensure_ascii=False)[:200]}")
out()

out("[7] HIGH を却下する")
rejected = invoke("POST", f"/api/requests/{high_id}/decision", {"verdict": "reject"}, approver="suwa")
show("結果", rejected["body"], ["verdict", "reviewer", "at"])
out()

out("[8] 監査ログ（自動承認された LOW も残っている）")
audit = invoke("GET", "/api/audit")
entries = audit["body"]
out(f"    件数: {len(entries)}")
for entry in entries:
    out(f"    - {json.dumps(entry, ensure_ascii=False)[:200]}")
out()

out("[9] 採用した設定値")
policy = invoke("GET", "/api/policy")
show("policy", policy["body"], ["version", "weights", "thresholds"])
out()
out("== 通し確認 完了 ==")

EVIDENCE.parent.mkdir(parents=True, exist_ok=True)
EVIDENCE.write_text("\n".join(lines) + "\n", encoding="utf-8")
print(f"\n記録: {EVIDENCE}")
