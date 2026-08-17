//! 承認ゲートの HTTP サーバ。
//!
//! ```bash
//! cargo run -p gate-api                 # http://127.0.0.1:8090
//! GATE_POLICY=policies/provisional.toml cargo run -p gate-api
//! ```
//!
//! # なぜ axum か
//!
//! `lambda_http` が axum の `Service` をそのまま受け取れるので、
//! **第7段階（Lambda）でハンドラを1行も変えずに済みます**。
//! ローカルは `axum::serve`、AWS は `lambda_http::run`。分岐はここだけです。

use std::net::SocketAddr;
use std::sync::Arc;

use gate_core::policy::Policy;
use tower_http::cors::CorsLayer;

use gate_api::routes;
use gate_api::state::AppState;
use gate_store::dynamo::{ABANDON_AFTER_SECONDS, DynamoStore};
use gate_store::{InMemoryStore, RequestStore};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "gate_api=info,tower_http=info".into()),
        )
        .init();

    // 設定ファイルがあれば読む。無ければ暫定値。
    //
    // 【重要】ファイルを開くのはここ（app 側）の仕事です。
    // gate_core は文字列を受け取るだけで、ファイルシステムを知りません。
    let policy = match std::env::var("GATE_POLICY") {
        Ok(path) => match std::fs::read_to_string(&path) {
            Ok(text) => match Policy::from_toml(&text) {
                Ok(p) => {
                    tracing::info!("設定を読みました: {path}（{}）", p.version);
                    p
                }
                Err(e) => {
                    // 【重要】壊れた設定で起動しない。
                    // 閾値が逆さまのまま動くと「なぜか承認待ちが出ない」形で表面化する。
                    eprintln!("設定が正しくありません: {}", e.message());
                    std::process::exit(2);
                }
            },
            Err(e) => {
                eprintln!("設定を読めません（{path}）: {e}");
                std::process::exit(2);
            }
        },
        Err(_) => {
            // 第4段階で確定した採用値。設定ファイルを渡さなければこれで動く。
            let p = Policy::adopted();
            tracing::info!("採用値で動いています（{}・{}）", p.version, p.label);
            p
        }
    };

    // 置き場を選ぶ。ハンドラは trait しか知らないので、変わるのはここだけ。
    //
    //   GATE_STORE=dynamodb  DynamoDB（ローカルなら GATE_DYNAMO_ENDPOINT も指定）
    //   （未指定）           メモリ。第3〜5段階と同じ
    let mut store_kind = "in-memory";
    let store: Arc<dyn RequestStore> = match std::env::var("GATE_STORE").as_deref() {
        Ok("dynamodb") => {
            let client = gate_store::connect().await;
            if let Err(e) = gate_store::ensure_tables(&client).await {
                eprintln!("テーブルを用意できません: {e}");
                std::process::exit(2);
            }
            let store = Arc::new(DynamoStore::new(client));
            store_kind = "dynamodb";
            tracing::info!("DynamoDB に保存します");
            spawn_sweeper(store.clone());
            store
        }
        _ => {
            tracing::warn!("メモリに保存します。落とすと消えます（GATE_STORE=dynamodb で永続化）");
            Arc::new(InMemoryStore::new())
        }
    };

    let state = Arc::new(AppState::new(store, policy).with_store_kind(store_kind));
    let app = routes::router(state).layer(CorsLayer::permissive());

    // ポートは 8090。8080 は別のプロジェクト（mendan-training）が使っているため、
    // ぶつからないようずらしてある。GATE_ADDR で変えられる。
    let addr: SocketAddr = std::env::var("GATE_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8090".to_string())
        .parse()
        .expect("GATE_ADDR の形が正しくありません");

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            // 【重要】黙って落ちない。ポートが埋まっているのはよくある。
            eprintln!("{addr} を開けません: {e}");
            eprintln!("別のポートで動かすには GATE_ADDR=127.0.0.1:9000 を指定してください");
            std::process::exit(2);
        }
    };
    tracing::info!("承認ゲートを開始しました: http://{addr}");
    tracing::info!("承認は X-Approver-Id ヘッダで名乗ります（認証は意図的に未実装）");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await
        .expect("サーバが落ちました");
}

/// 放置された承認待ちを、期限切れとして閉じる。
///
/// <div class="warning">
///
/// 【重要】これが無いと、承認待ちのまま放置された平文が永久に残ります
/// （諏訪の指示・第6段階）。TTL は「判断が下りてから」動き出すので、
/// 判断が下りない要求には、いつまでも時計が動きません。
///
/// mensetsu の StaleSessionSweeper と同じ形です。切断や放置は必ず起きるので、
/// 拾えなかったものを後から片付ける係が要ります。
///
/// </div>
fn spawn_sweeper(store: Arc<DynamoStore>) {
    tokio::spawn(async move {
        // 起動時に1回、そのあとは1時間ごと。
        loop {
            let now = chrono::Utc::now().to_rfc3339();
            let closed = store.expire_abandoned(ABANDON_AFTER_SECONDS, &now).await;
            if closed > 0 {
                tracing::info!(
                    "{}日以上放置された承認待ち {}件を期限切れとして閉じました",
                    ABANDON_AFTER_SECONDS / 86_400,
                    closed
                );
            }
            tokio::time::sleep(std::time::Duration::from_secs(3_600)).await;
        }
    });
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("停止します");
}
