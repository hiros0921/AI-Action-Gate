//! 「誰が承認しようとしているか」を HTTP から取り出す。
//!
//! <div class="warning">
//!
//! 【重要】認証は**意図的に実装していません**（仕様書1章・10章）。
//!
//! ヘッダ `X-Approver-Id` の値をそのまま承認者として扱います。誰でも名乗れます。
//! これはプロトタイプとして意図した割り切りで、隠す気はありません。
//!
//! そのうえで、**差し替えられる形**にしてあります。
//! Cognito でも JWT でも社内SSOでも、**このファイルだけを書き換えれば済みます**。
//! ドメイン層（`gate_core::review`）は `approver_id` を文字列として受け取るだけで、
//! それがどこから来たのかを知りません。
//!
//! > 認証を作らないことと、認証を後から入れられないことは、別です。（諏訪）
//!
//! </div>

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use gate_core::review::{ApproverError, Reviewer, human};

/// 承認者を名乗るヘッダ。
pub const APPROVER_HEADER: &str = "x-approver-id";

/// 承認者。ハンドラの引数に書けば、ヘッダから取り出される。
#[derive(Debug, Clone)]
pub struct Approver(pub Reviewer);

impl<S> FromRequestParts<S> for Approver
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let raw = parts
            .headers
            .get(APPROVER_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();

        // 【重要】ここで弾く。空の承認者IDで監査ログを書かせない。
        match human(raw) {
            Ok(reviewer) => Ok(Approver(reviewer)),
            Err(ApproverError::Empty) => Err((
                StatusCode::UNAUTHORIZED,
                format!("{APPROVER_HEADER} ヘッダで承認者IDを指定してください"),
            )),
            Err(e) => Err((StatusCode::BAD_REQUEST, e.message())),
        }
    }
}
