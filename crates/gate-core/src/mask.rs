//! 伏せ字。
//!
//! <div class="warning">
//!
//! 【重要】「どこを伏せるか」と「伏せた文字列を作る」を分けてあります。
//!
//! [`plan`] が置換の一覧（[`MaskPlan`]）を返し、[`MaskPlan::apply`] が文字列を作ります。
//! 分けておくと、承認画面で「この1件は人名ではないので外す」ができます。
//! 1関数で文字列を返す作りにすると、外すには全部やり直すしかありません。
//!
//! </div>

use crate::detect::{Finding, PiiKind, Scan};

/// 1か所ぶんの置換。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    pub start: usize,
    pub end: usize,
    pub kind: PiiKind,
    /// 置き換えたあとの文字列。
    pub masked: String,
}

/// 置換の計画。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MaskPlan {
    pub replacements: Vec<Replacement>,
}

impl MaskPlan {
    /// 適用して伏せ字の文字列を作る。
    pub fn apply(&self, text: &str) -> String {
        let mut sorted: Vec<&Replacement> = self.replacements.iter().collect();
        sorted.sort_by_key(|r| r.start);

        let mut out = String::with_capacity(text.len());
        let mut cursor = 0;
        for r in sorted {
            if r.start < cursor {
                continue; // 重なりは先に採った側を残す
            }
            out.push_str(&text[cursor..r.start]);
            out.push_str(&r.masked);
            cursor = r.end;
        }
        out.push_str(&text[cursor..]);
        out
    }

    /// 承認画面で1件外す。
    pub fn without(&self, start: usize) -> MaskPlan {
        MaskPlan {
            replacements: self
                .replacements
                .iter()
                .filter(|r| r.start != start)
                .cloned()
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.replacements.is_empty()
    }
}

/// 検出結果から置換の計画を作る。
pub fn plan(text: &str, scan: &Scan) -> MaskPlan {
    MaskPlan {
        replacements: scan
            .findings()
            .iter()
            .map(|f| Replacement {
                start: f.start,
                end: f.end,
                kind: f.kind,
                masked: mask_of(&text[f.start..f.end], f),
            })
            .collect(),
    }
}

/// 伏せ方。
///
/// 【重要】形を残します。全部を `***` にすると、承認する人が
/// 「何を伏せたのか」を確かめられなくなります。桁数や区切りが残っていれば、
/// 電話番号なのか郵便番号なのかが伏せたあとでも分かります。
fn mask_of(matched: &str, finding: &Finding) -> String {
    match finding.kind {
        // メールは記号を残す。`○○○○@○○○○.com` の形。
        PiiKind::Email => matched
            .chars()
            .map(|c| if c == '@' || c == '.' { c } else { '○' })
            .collect(),
        // それ以外は、区切りと単位（年月日）を残して中身だけ伏せる。
        _ => matched
            .chars()
            .map(|c| if is_structural(c) { c } else { '○' })
            .collect(),
    }
}

/// 伏せずに残す文字。
fn is_structural(c: char) -> bool {
    matches!(
        c,
        '-' | '‐' | '−' | '/' | '.' | ' ' | '　' | '年' | '月' | '日' | '〒' | '+' | '・'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::{DetectConfig, scan};

    #[test]
    fn 仕様書の例と同じ形になること() {
        // 仕様書2章の応答例:
        //   "○○○○さん（○○○○年○月○日生・○○○-○○○○-○○○○）の検査結果について..."
        let text = "山田太郎さん（1980年3月15日生・090-1234-5678）の検査結果について...";
        let s = scan(text, &DetectConfig::default());
        let masked = plan(text, &s).apply(text);

        // 【重要】仕様書の応答例は "○○○○年○月○日生" ですが、こちらは
        // "○○○○年○月○○日生" になります。15 を1文字に潰さず、桁数を残すためです。
        //
        // 仕様書の例も、電話番号は "○○○-○○○○-○○○○" と桁を残しています。
        // 日付だけ潰すと不揃いになるので、桁を残す側に揃えました。
        // 承認する人が「これは本当に日付か」を伏せたあとでも確かめられます。
        // 潰す形が要るようでしたら直します。
        assert!(
            masked.contains("○○○○年○月○○日生"),
            "日付の形が残っていない: {masked}"
        );
        assert!(
            masked.contains("○○○-○○○○-○○○○"),
            "電話の形が残っていない: {masked}"
        );
        assert!(!masked.contains("山田"), "氏名が残っている: {masked}");
        assert!(!masked.contains("1980"), "生年が残っている: {masked}");
        assert!(!masked.contains("5678"), "電話番号が残っている: {masked}");
        assert!(
            masked.contains("の検査結果について"),
            "本文まで消している: {masked}"
        );
    }

    #[test]
    fn メールは形を残して伏せること() {
        let text = "連絡先は taro@example.com です";
        let s = scan(text, &DetectConfig::default());
        let masked = plan(text, &s).apply(text);
        assert!(masked.contains("@"), "区切りまで消している: {masked}");
        assert!(!masked.contains("taro"), "ローカル部が残っている: {masked}");
        assert!(
            !masked.contains("example"),
            "ドメインが残っている: {masked}"
        );
    }

    #[test]
    fn 一件だけ外せること() {
        // 承認画面で「これは人名ではない」と判断できるようにするため。
        let text = "山田太郎さんの電話は090-1234-5678です";
        let s = scan(text, &DetectConfig::default());
        let full = plan(text, &s);
        assert!(full.replacements.len() >= 2);

        let name_at = full
            .replacements
            .iter()
            .find(|r| r.kind == PiiKind::PersonName)
            .unwrap()
            .start;
        let masked = full.without(name_at).apply(text);

        assert!(
            masked.contains("山田太郎"),
            "外した1件まで伏せている: {masked}"
        );
        assert!(!masked.contains("5678"), "残りが伏せられていない: {masked}");
    }

    #[test]
    fn 検出が無ければ元のままであること() {
        let text = "在庫の確認をお願いします";
        let s = scan(text, &DetectConfig::default());
        let masked = plan(text, &s).apply(text);
        assert_eq!(masked, text);
    }

    #[test]
    fn 走査していない本文は伏せ字を作らないこと() {
        // 【重要】ここで空の計画が返るのは正しい。
        // ただし「PIIが無い」という意味ではないので、承認側は Scan を見ること。
        let cfg = DetectConfig {
            payload_limit: 4,
            ..Default::default()
        };
        let text = "山田太郎さんの検査結果";
        let s = scan(text, &cfg);
        assert!(plan(text, &s).is_empty());
        assert!(!s.is_conclusive(), "走査していないことが分かる形であること");
    }
}
