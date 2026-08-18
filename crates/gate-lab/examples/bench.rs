//! 判定1回にかかる時間を測る。README の「なぜ Rust か」の根拠にする。
fn main() {
    use gate_core::action::*;
    use gate_core::detect::{DetectConfig, scan};
    use gate_core::policy::Policy;
    use gate_core::score::assess;

    let policy = Policy::adopted();
    let cfg = DetectConfig::default();
    // 実運用に近い長さの本文（架空）。1KB 程度。
    let long = "山田太郎さん（1980年3月15日生・090-1234-5678）の検査結果について要約してください。連絡先は taro@example.com、住所は〒123-4567 です。".repeat(8);

    // 上限いっぱい（64KB）。正規表現を全文に走らせるので、ここが最悪値になる。
    let huge = "山田太郎さん（1980年3月15日生・090-1234-5678）の検査結果。".repeat(700);
    for (label, text) in [
        (
            "短い本文（60字）",
            "倉庫Aの在庫一覧を確認します。対象は全SKUです。",
        ),
        ("長い本文（約1KB）", long.as_str()),
        ("上限に近い本文", huge.as_str()),
    ] {
        let req = ActionRequest {
            agent_id: "agent-001".into(),
            action: ActionKind::SendExternal,
            destination: Destination::ExternalAi,
            data_class: DataClass::PersonalInformation,
            payload: Payload {
                text: text.to_string(),
            },
        };
        // 温める
        for _ in 0..100 {
            let s = scan(text, &cfg);
            assess(&req, &s, &policy);
        }

        let n = if text.len() > 10_000 { 100 } else { 2000 };
        let t = std::time::Instant::now();
        for _ in 0..n {
            let s = scan(text, &cfg);
            assess(&req, &s, &policy);
        }
        let each = t.elapsed() / n;
        println!(
            "{label:<20} {:>8.3} ms/件  （{}バイト）",
            each.as_secs_f64() * 1000.0,
            text.len()
        );
    }
}
