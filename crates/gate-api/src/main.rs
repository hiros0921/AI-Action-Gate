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
use gate_api::store::InMemoryStore;

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
            tracing::warn!("暫定の設定で動いています（第4段階で採用値に差し替えます）");
            Policy::provisional()
        }
    };

    let state = Arc::new(AppState::new(Arc::new(InMemoryStore::new()), policy));
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

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("停止します");
}
