//! 承認ゲートの HTTP 層。
//!
//! 【重要】判定は何ひとつここにありません。すべて `gate_core` の側です。
//! ここがするのは、受け取る・渡す・保存する・返すだけ。
//!
//! bin と lib に分けてあるのは、**ハンドラを試験から呼べるようにする**ためです。
//! サーバを起動せずに `tower::ServiceExt::oneshot` で叩けます。
pub mod approver;
pub mod routes;
pub mod state;
pub mod store;
