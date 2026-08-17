//! 疑似エージェント。
//!
//! 本物の AI エージェントは作りません（仕様書10章）。
//! ここがするのは「JSON を投げて、request_id で結果を取りに行く」ことだけです。
//!
//! ```bash
//! cargo run -p gate-agent -- send --scenario low
//! cargo run -p gate-agent -- send --scenario high --wait
//! cargo run -p gate-agent -- status req-1a2b3c4d
//! ```
//!
//! # なぜ Rust の CLI か
//!
//! 同じ workspace に置けるので、**要求と応答の型を gate-core と共有できます**。
//! 追加のランタイムも要りません。curl でも同じことができるので、
//! 生の形は README に置いてあります。

use clap::{Parser, Subcommand, ValueEnum};
use gate_core::action::{ActionKind, ActionRequest, DataClass, Destination, Payload};

#[derive(Parser)]
#[command(
    name = "gate-agent",
    about = "承認ゲートに実行要求を投げる疑似エージェント"
)]
struct Cli {
    /// 承認ゲートの場所。
    #[arg(long, default_value = "http://127.0.0.1:8090", global = true)]
    url: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 要求を投げる。
    Send {
        /// あらかじめ用意した3経路のどれか。
        #[arg(long, value_enum, default_value_t = Scenario::Low)]
        scenario: Scenario,
        /// 本文を自分で指定する（scenario より優先）。
        #[arg(long)]
        text: Option<String>,
        /// 承認が下りるまで待つ。
        #[arg(long)]
        wait: bool,
    },
    /// 結果を取りに行く。
    Status { request_id: String },
}

/// LOW / MEDIUM / HIGH の3経路。
///
/// 【重要】本文はすべて架空です（仕様書9章）。実在の人物・企業を含みません。
#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Scenario {
    /// 社内の読み取り。PII なし。自動承認される想定。
    Low,
    /// 社外への書き込み。PII は少ない。承認待ちになる想定。
    Medium,
    /// 外部AIへ個人情報を送る。マスキング案つきで承認待ちになる想定。
    High,
}

impl Scenario {
    fn request(self) -> ActionRequest {
        match self {
            Self::Low => ActionRequest {
                agent_id: "agent-001".into(),
                action: ActionKind::Read,
                destination: Destination::Internal,
                data_class: DataClass::Public,
                payload: Payload {
                    text: "在庫一覧を確認します。対象は倉庫Aの全SKUです。".into(),
                },
            },
            Self::Medium => ActionRequest {
                agent_id: "agent-001".into(),
                action: ActionKind::Write,
                destination: Destination::External,
                data_class: DataClass::Internal,
                payload: Payload {
                    text: "取引先へ発注書を送信します。連絡先は order@example.com です。".into(),
                },
            },
            Self::High => ActionRequest {
                agent_id: "agent-001".into(),
                action: ActionKind::SendExternal,
                destination: Destination::ExternalAi,
                data_class: DataClass::PersonalInformation,
                payload: Payload {
                    text: "山田太郎さん（1980年3月15日生・090-1234-5678）の検査結果について要約してください。".into(),
                },
            },
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Low => "LOW（社内の読み取り・PIIなし）",
            Self::Medium => "MEDIUM（社外への書き込み・メールアドレスあり）",
            Self::High => "HIGH（外部AIへ個人情報）",
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let http = reqwest::Client::new();

    match cli.command {
        Command::Send {
            scenario,
            text,
            wait,
        } => {
            let mut request = scenario.request();
            if let Some(t) = text {
                request.payload.text = t;
            }
            println!("── 要求を投げます: {} ──", scenario.label());
            println!("{}\n", serde_json::to_string_pretty(&request)?);

            let response: serde_json::Value = http
                .post(format!("{}/api/requests", cli.url))
                .json(&request)
                .send()
                .await?
                .json()
                .await?;
            println!("{}", serde_json::to_string_pretty(&response)?);

            let Some(id) = response.get("request_id").and_then(|v| v.as_str()) else {
                return Ok(());
            };
            if wait {
                println!("\n── 承認を待ちます（{id}）──");
                poll(&http, &cli.url, id).await?;
            }
        }
        Command::Status { request_id } => {
            let response: serde_json::Value = http
                .get(format!("{}/api/requests/{request_id}", cli.url))
                .send()
                .await?
                .json()
                .await?;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
    }
    Ok(())
}

/// 結果が出るまで request_id で聞きに行く。
///
/// 【重要】SQS を挟まないのは、これで足りるからです（仕様書6章）。
/// 非同期化が要るほどの量にならないうちにキューを挟むと、
/// 動かす部品だけが増えます。
async fn poll(
    http: &reqwest::Client,
    url: &str,
    id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    for _ in 0..120 {
        let response: serde_json::Value = http
            .get(format!("{url}/api/requests/{id}"))
            .send()
            .await?
            .json()
            .await?;
        let status = response
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        if status != "pending" {
            println!("{}", serde_json::to_string_pretty(&response)?);
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    println!("2分待ちましたが、まだ承認されていません。");
    Ok(())
}
