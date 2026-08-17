//! AI Action Gate の判定コア。
//!
//! エージェントの実行要求を受けて、**通してよいか・人に見せるべきか**を決める。
//!
//! # ここに I/O はありません
//!
//! HTTP も DB も AWS も、このクレートは知りません。依存にも入れていません
//! （`Cargo.toml` を参照）。理由は2つあります。
//!
//! - **判定の試験にサーバも DB も要らない。** `cargo test` だけで完結します。
//!   起動に数秒かかる試験は、細かい条件を並べて書く気がなくなります。
//!   書かれなかった条件は、そのまま確かめていない条件になります。
//! - **閾値シミュレーションが成立する。** 判定が純粋関数なので、
//!   同じ入力なら必ず同じ点になります。時刻や外部状態が混ざった瞬間、
//!   「過去の要求を新しい閾値で再判定する」が嘘になります。
//!
//! # 流れ
//!
//! ```
//! use gate_core::{action::*, detect, policy::Policy, score};
//!
//! let request = ActionRequest {
//!     agent_id: "agent-001".into(),
//!     action: ActionKind::SendExternal,
//!     destination: Destination::ExternalAi,
//!     data_class: DataClass::PersonalInformation,
//!     payload: Payload { text: "山田太郎さん（1980年3月15日生）の件".into() },
//! };
//!
//! // ① 形式の検証
//! request.validate(64 * 1024).expect("形式は正しい");
//!
//! // ② PII の検出（見つけるだけ）
//! let scan = detect::scan(&request.payload.text, &detect::DetectConfig::default());
//!
//! // ③ 配点（純粋関数）
//! let assessment = score::assess(&request, &scan, &Policy::provisional());
//!
//! // ④ 伏せ字（承認画面のプレビュー用）
//! let preview = gate_core::mask::plan(&request.payload.text, &scan).apply(&request.payload.text);
//!
//! assert!(assessment.needs_human());
//! assert!(!preview.contains("山田"));
//! ```
//!
//! # 分け方の理由
//!
//! `detect` / `score` / `mask` を分けてあるのは、**変える理由が違う**からです。
//!
//! - `detect` が変わるのは、検出漏れが見つかったとき
//! - `score` が変わるのは、運用が「ここは通しすぎだ」と感じたとき
//! - `mask` が変わるのは、伏せ方の見た目を直したいとき
//!
//! 同じ場所に置くと、閾値を触るたびに正規表現の差分が混ざります。

pub mod action;
pub mod detect;
pub mod mask;
pub mod policy;
pub mod score;

pub use action::{ActionKind, ActionRequest, DataClass, Destination, Payload};
pub use detect::{Confidence, PiiKind, Scan};
pub use policy::{Decision, Policy, Thresholds};
pub use score::{RiskAssessment, assess};
