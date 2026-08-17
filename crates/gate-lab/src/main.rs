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
use gate_core::policy::Policy;
use gate_core::score::assess;

mod samples;

use samples::{Expectation, Kind};

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
}

fn main() {
    match Cli::parse().command {
        Command::Samples => list_samples(),
    }
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
                "  {:<6} {:>5} {:>5} {:<8} {:<28} {}",
                "ID", "生合計", "点", "判定", "検出", "内容"
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
