//! 案を比べるための道具。
//!
//! ```bash
//! cargo run -p gate-lab -- samples      # 比較に使う材料を一覧する
//! ```
//!
//! 【重要】案を比べる前に、**材料の妥当性を先に確認します**（諏訪の指示・第4段階）。
//! 材料が偏っていると、どの案を選んでも判断がずれます。

use clap::{Parser, Subcommand};
use gate_core::detect;
use gate_core::policy::{Decision, Policy};
use gate_core::score::assess;

mod proposals;
mod samples;

use samples::{Expectation, Kind, Sample};

#[derive(Parser)]
#[command(name = "gate-lab", about = "スコアリング案を実測で比べる")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 比較に使うサンプル要求の一覧。
    Samples,
    /// 重み案の比較（閾値は固定）。
    Weights,
    /// 閾値案の比較（重みは採用済みの案W2に固定）。
    Thresholds,
}

fn main() {
    match Cli::parse().command {
        Command::Samples => list_samples(),
        Command::Weights => compare_weights(),
        Command::Thresholds => compare_thresholds(),
    }
}

/// 案1つぶんの成績。
struct Report {
    /// 【最優先】危険な要求（型2・型4）が自動承認された件数。0でなければ不採用。
    dangerous_auto: Vec<String>,
    /// 安全な要求（型1）が承認待ちに回った件数。少ないほうが良い。
    safe_held: Vec<String>,
    /// 承認待ちの総件数。
    held_total: usize,
    /// 型ごとの点数分布。
    spread: Vec<(Kind, u8, u8, u8)>,
    /// 型ごとの判定内訳。
    decisions: Vec<(Kind, usize, usize, usize)>,
}

fn evaluate(all: &[Sample], policy: &Policy) -> Report {
    let mut dangerous_auto = Vec::new();
    let mut safe_held = Vec::new();
    let mut held_total = 0;
    let mut spread = Vec::new();
    let mut decisions = Vec::new();

    let mut kinds: Vec<Kind> = all.iter().map(|s| s.kind).collect();
    kinds.sort();
    kinds.dedup();

    for kind in kinds {
        let mut scores = Vec::new();
        let (mut low, mut medium, mut high) = (0, 0, 0);

        for s in all.iter().filter(|s| s.kind == kind) {
            let scan = detect::scan(&s.request.payload.text, &s.detect);
            let a = assess(&s.request, &scan, policy);
            scores.push(a.score);
            match a.decision {
                Decision::Low => low += 1,
                Decision::Medium => medium += 1,
                Decision::High => high += 1,
            }
            if a.decision.needs_human() {
                held_total += 1;
            }
            match kind.expectation() {
                // 【最優先】ここが破れた案は、他がどれだけ良くても採れない。
                Expectation::MustHold if a.decision == Decision::Low => {
                    dangerous_auto.push(format!("{}({}点)", s.id, a.score));
                }
                Expectation::ShouldPassAuto if a.decision.needs_human() => {
                    safe_held.push(format!("{}({}点)", s.id, a.score));
                }
                _ => {}
            }
        }

        scores.sort_unstable();
        let n = scores.len();
        spread.push((kind, scores[0], scores[n / 2], scores[n - 1]));
        decisions.push((kind, low, medium, high));
    }

    Report {
        dangerous_auto,
        safe_held,
        held_total,
        spread,
        decisions,
    }
}

fn compare_weights() {
    let all = samples::all();
    let proposals = proposals::weight_proposals();

    line();
    println!("  重み案の比較（{}件のサンプル）", all.len());
    println!(
        "  【重要】閾値は {}/{} に固定してあります。重みと閾値を同時に変えると、",
        proposals::FIXED_THRESHOLDS.medium_at,
        proposals::FIXED_THRESHOLDS.high_at
    );
    println!("  差がどちらから来たのか分からなくなります（諏訪の指示）");
    line();
    for p in &proposals {
        println!(
            "  案{}・{:<10} 操作{:>3} 送信先{:>3} 区分{:>3} PII{:>3}",
            p.id,
            p.label,
            p.weights.action,
            p.weights.destination,
            p.weights.data_class,
            p.weights.pii
        );
        println!("      {}", p.stance);
    }

    let reports: Vec<(&proposals::WeightProposal, Report)> = proposals
        .iter()
        .map(|p| (p, evaluate(&all, &p.policy())))
        .collect();

    line();
    println!("  ① 危険な要求（型2・型4）が自動承認された件数 ← 必ず0であること");
    line();
    for (p, r) in &reports {
        println!(
            "  案{}  {:>2}件  {}",
            p.id,
            r.dangerous_auto.len(),
            if r.dangerous_auto.is_empty() {
                "✓".to_string()
            } else {
                format!("✗ {}", r.dangerous_auto.join("・"))
            }
        );
    }

    line();
    println!("  ② 安全な要求（型1）が承認待ちに回った件数 ← 少ないほうが良い");
    println!("  ③ 承認待ちの総件数（{}件中）", all.len());
    line();
    println!(
        "  {:<6} {:>10} {:>12} {:>10}",
        "案", "型1が回った", "承認待ち総数", "人手の割合"
    );
    for (p, r) in &reports {
        println!(
            "  案{:<5} {:>10} {:>12} {:>9}%",
            p.id,
            format!("{}件", r.safe_held.len()),
            format!("{}件", r.held_total),
            (r.held_total * 100).div_ceil(all.len())
        );
    }

    line();
    println!("  ④ 型ごとの点数分布（最小 / 中央 / 最大）と判定の内訳（LOW-MED-HIGH）");
    line();
    print!("  {:<22}", "型");
    for (p, _) in &reports {
        print!("{:<22}", format!("案{}", p.id));
    }
    println!();
    for (i, (kind, _, _, _)) in reports[0].1.spread.iter().enumerate() {
        print!("  型{} {:<18}", kind.number(), kind.label());
        for (_, r) in &reports {
            let (_, min, mid, max) = r.spread[i];
            let (_, low, med, high) = r.decisions[i];
            print!(
                "{:<22}",
                format!("{min:>3}/{mid:>3}/{max:>3}  {low}-{med}-{high}")
            );
        }
        println!();
    }

    line();
    println!("  ⑤ 次の段階（閾値）に残る自由度");
    println!("  型1の最大点より上、型2の最小点以下——ここが閾値を置ける範囲です。");
    println!("  狭い案を選ぶと、閾値の選択肢がその時点で減ります。");
    line();
    println!(
        "  {:<6} {:>10} {:>10} {:>18}",
        "案", "型1の最大", "型2の最小", "置ける範囲"
    );
    for (p, r) in &reports {
        let safe_max = r
            .spread
            .iter()
            .find(|(k, ..)| *k == Kind::InternalRead)
            .unwrap()
            .3;
        let danger_min = r
            .spread
            .iter()
            .find(|(k, ..)| *k == Kind::PatientToExternalAi)
            .unwrap()
            .1;
        println!(
            "  案{:<5} {:>10} {:>10} {:>18}",
            p.id,
            format!("{safe_max}点"),
            format!("{danger_min}点"),
            format!(
                "{}〜{} （{}点幅）",
                safe_max + 1,
                danger_min,
                danger_min - safe_max
            )
        );
    }

    line();
    println!("  ⑥ 案によって分かれた要求（型3・5・6・7）");
    println!("  ここが動くところです。型1と型2は、どの案でも動きません。");
    line();
    print!("  {:<10}", "ID");
    for (p, _) in &reports {
        print!("{:<14}", format!("案{}", p.id));
    }
    println!("内容");
    for s in all
        .iter()
        .filter(|s| s.kind.expectation() == Expectation::Split)
    {
        let mut cells = Vec::new();
        let mut differs = false;
        let mut first: Option<Decision> = None;
        for (p, _) in &reports {
            let policy = p.policy();
            let scan = detect::scan(&s.request.payload.text, &s.detect);
            let a = assess(&s.request, &scan, &policy);
            if first.is_none() {
                first = Some(a.decision);
            } else if first != Some(a.decision) {
                differs = true;
            }
            cells.push(format!("{:>3}点 {:?}", a.score, a.decision));
        }
        // 動いたものに印を付ける。全案で同じものは、選定の材料にならない。
        print!(
            "  {:<10}",
            format!("{}{}", s.id, if differs { " *" } else { "" })
        );
        for c in cells {
            print!("{c:<14}");
        }
        println!("{}", s.note);
    }
    println!();
    println!("  * 案によって判定が変わったもの");
    line();
}

fn list_samples() {
    let policy = Policy::provisional();
    let all = samples::all();

    line();
    println!("  比較に使うサンプル要求（{}件）", all.len());
    println!(
        "  点数は暫定の設定（{}・重み25/25/25/25・閾値30/70）で採点したもの",
        policy.version
    );
    println!("  【重要】この点数は材料の位置を見るためのものです。採用値ではありません");
    line();

    let mut current: Option<Kind> = None;
    for s in &all {
        if current != Some(s.kind) {
            current = Some(s.kind);
            println!();
            println!(
                "  ── 型{}: {} ／ 期待: {} ──",
                s.kind.number(),
                s.kind.label(),
                match s.kind.expectation() {
                    Expectation::ShouldPassAuto => "自動承認されてよい",
                    Expectation::MustHold => "必ず承認待ち（破れた案は不採用）",
                    Expectation::Split => "案によって分かれてよい",
                }
            );
            println!(
                "  {:<6} {:>5} {:>5} {:<8} {:<28} 内容",
                "ID", "生合計", "点", "判定", "検出"
            );
        }

        let scan = detect::scan(&s.request.payload.text, &s.detect);
        let a = assess(&s.request, &scan, &policy);

        let detected = if !scan.is_conclusive() {
            "（走査できず）".to_string()
        } else if a.detected.is_empty() {
            "なし".to_string()
        } else {
            a.detected
                .iter()
                .map(|d| {
                    format!(
                        "{}{}",
                        d.kind.label(),
                        if d.count > 1 {
                            format!("×{}", d.count)
                        } else {
                            String::new()
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("・")
        };

        println!(
            "  {:<6} {:>5} {:>5} {:<8} {:<28} {}",
            s.id,
            a.raw_total,
            a.score,
            format!(
                "{:?}{}",
                a.decision,
                if a.raised_by_uncertainty { "*" } else { "" }
            ),
            truncate(&detected, 28),
            s.note,
        );
    }

    println!();
    println!("  * 走査できなかったため引き上げたもの（点では LOW だが人手に回る）");
    line();
    summary(&all, &policy);
}

fn summary(all: &[samples::Sample], policy: &Policy) {
    println!("  材料の偏りを確かめる");
    line();

    let mut kinds: Vec<Kind> = all.iter().map(|s| s.kind).collect();
    kinds.dedup();

    for kind in kinds {
        let scores: Vec<u8> = all
            .iter()
            .filter(|s| s.kind == kind)
            .map(|s| {
                let scan = detect::scan(&s.request.payload.text, &s.detect);
                assess(&s.request, &scan, policy).score
            })
            .collect();
        let n = scores.len();
        let mut sorted = scores.clone();
        sorted.sort_unstable();
        println!(
            "  型{} {:<18} {}件   最小{:>3} 中央{:>3} 最大{:>3}",
            kind.number(),
            kind.label(),
            n,
            sorted[0],
            sorted[n / 2],
            sorted[n - 1],
        );
    }

    println!();
    println!("  【重要】案の差が出るのは中間帯だけです。");
    println!("  素点100（頭打ち）と底に寄った要求は、閾値を動かしても分岐が変わりません。");
    line();
}

fn truncate(s: &str, width: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= width {
        s.to_string()
    } else {
        chars[..width - 1].iter().collect::<String>() + "…"
    }
}

fn line() {
    println!("{}", "=".repeat(108));
}

fn compare_thresholds() {
    let all = samples::all();
    let proposals = proposals::threshold_proposals();
    let w = proposals::ADOPTED_WEIGHTS;

    line();
    println!("  閾値案の比較（{}件のサンプル）", all.len());
    println!(
        "  【重要】重みは採用済みの案W2に固定してあります（操作{} 送信先{} 区分{} PII{}）",
        w.action, w.destination, w.data_class, w.pii
    );
    println!("  諏訪の申し送り: 13〜25の範囲を優先。初期値は安全側でよい");
    line();
    for p in &proposals {
        println!(
            "  案{}・{:<12} MEDIUM {:>3}以上 / HIGH {:>3}以上{}",
            p.id,
            p.label,
            p.thresholds.medium_at,
            p.thresholds.high_at,
            if (13..=25).contains(&p.thresholds.medium_at) {
                ""
            } else {
                "   ← 推奨範囲の外"
            }
        );
        println!("      {}", p.stance);
    }

    let reports: Vec<(&proposals::ThresholdProposal, Report)> = proposals
        .iter()
        .map(|p| (p, evaluate(&all, &p.policy())))
        .collect();

    line();
    println!("  ① 危険な要求（型2・型4）が自動承認された件数 ← 必ず0であること");
    line();
    for (p, r) in &reports {
        println!(
            "  案{}  {:>2}件  {}",
            p.id,
            r.dangerous_auto.len(),
            if r.dangerous_auto.is_empty() {
                "✓".to_string()
            } else {
                format!("✗ {}", r.dangerous_auto.join("・"))
            }
        );
    }

    line();
    println!("  ② 安全な要求（型1）が承認待ちに回った件数 ← 少ないほうが良い");
    println!("  ③ 承認待ちの総件数（{}件中）", all.len());
    line();
    println!(
        "  {:<6} {:>10} {:>12} {:>10} {:>12}",
        "案", "型1が回った", "承認待ち総数", "人手の割合", "うちHIGH"
    );
    for (p, r) in &reports {
        let high: usize = r.decisions.iter().map(|(_, _, _, h)| h).sum();
        println!(
            "  案{:<5} {:>10} {:>12} {:>9}% {:>12}",
            p.id,
            format!("{}件", r.safe_held.len()),
            format!("{}件", r.held_total),
            (r.held_total * 100).div_ceil(all.len()),
            format!("{high}件"),
        );
    }

    line();
    println!("  ④ 型ごとの判定内訳（LOW-MED-HIGH）。点数は重みが同じなので全案共通");
    line();
    print!("  {:<24}{:>6}", "型", "点");
    for (p, _) in &reports {
        print!("{:>10}", format!("案{}", p.id));
    }
    println!();
    for (i, (kind, min, mid, max)) in reports[0].1.spread.iter().enumerate() {
        print!(
            "  型{} {:<20}{:>6}",
            kind.number(),
            kind.label(),
            format!("{min}/{mid}/{max}")
        );
        for (_, r) in &reports {
            let (_, low, med, high) = r.decisions[i];
            print!("{:>10}", format!("{low}-{med}-{high}"));
        }
        println!();
    }

    line();
    println!("  ⑤ 案によって判定が変わった要求");
    line();
    for s in all
        .iter()
        .filter(|s| s.kind.expectation() != Expectation::MustHold)
    {
        let mut cells = Vec::new();
        let mut first: Option<Decision> = None;
        let mut differs = false;
        let mut score = 0;
        for (p, _) in &reports {
            let scan = detect::scan(&s.request.payload.text, &s.detect);
            let a = assess(&s.request, &scan, &p.policy());
            score = a.score;
            match first {
                None => first = Some(a.decision),
                Some(d) if d != a.decision => differs = true,
                _ => {}
            }
            cells.push(format!("{:?}", a.decision));
        }
        if !differs {
            continue;
        }
        print!("  {:<10}{:>4}点  ", s.id, score);
        for c in cells {
            print!("{c:<9}");
        }
        println!("{}", s.note);
    }

    line();
    println!("  ⑥ 25 から 40 へ緩めたら何が自動に落ちるか（諏訪の申し送り）");
    println!("  これは画面のシミュレーションで見せる形そのものです");
    line();
    let strict = proposals.iter().find(|p| p.id == "T3").unwrap().policy();
    let loose = proposals.iter().find(|p| p.id == "T4").unwrap().policy();
    let mut moved = Vec::new();
    for s in &all {
        let scan = detect::scan(&s.request.payload.text, &s.detect);
        let before = assess(&s.request, &scan, &strict);
        let after = assess(&s.request, &scan, &loose);
        if before.decision.needs_human() && !after.decision.needs_human() {
            moved.push((s, before.score, !after.detected.is_empty()));
        }
    }
    println!("  自動承認へ移るのは {}件", moved.len());
    for (s, score, with_pii) in &moved {
        println!(
            "    {:<10}{:>4}点  型{}  {}{}",
            s.id,
            score,
            s.kind.number(),
            s.note,
            if *with_pii { "   ← PIIを含む" } else { "" }
        );
    }
    line();
}
