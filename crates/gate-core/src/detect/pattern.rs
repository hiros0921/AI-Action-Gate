//! 形で見つかるもの。メール・電話番号・生年月日・郵便番号。
//!
//! <div class="warning">
//!
//! 【重要】この4つは互いにぶつかります。
//!
//! `123-4567` は郵便番号にも見えるし、電話番号の一部にも見えます。
//! `2024-03-15` は日付にも、なにかの整理番号にも見えます。
//!
//! そこで、**単独で確定できるものだけを High** にし、
//! 文脈語（〒・生・TEL など）が要るものは Medium 以下に置きます。
//! 重なりの解決は [`super::resolve_overlaps`] が行います。
//!
//! </div>

use std::sync::LazyLock;

use regex::Regex;

use super::{Confidence, Evidence, Finding, PiiKind};

/// メールアドレス。記号の並びが特徴的なので、単独で確定できる。
static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}").unwrap());

/// 日本の電話番号。市外局番の桁数が可変なので、区切りありを前提にする。
static PHONE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:\+81[\-\s]?|0)\d{1,4}[\-\s]\d{1,4}[\-\s]\d{3,4}").unwrap());

/// 区切りの無い携帯番号。11桁で 0[7-9]0 始まり。
static PHONE_PLAIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"0[789]0\d{8}").unwrap());

/// 西暦の日付。
static DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(19|20)\d{2}\s*[年/\-\.]\s*(1[0-2]|0?[1-9])\s*[月/\-\.]\s*(3[01]|[12]\d|0?[1-9])\s*日?",
    )
    .unwrap()
});

/// 郵便番号。〒があれば確定、無ければ文脈語を見る。
static POSTAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"〒?\s*\d{3}-\d{4}").unwrap());

/// 生年月日らしさを補強する語。
const BIRTH_CONTEXT: [&str; 5] = ["生年月日", "生まれ", "誕生日", "年生", "生・"];
/// 郵便番号らしさを補強する語。
const POSTAL_CONTEXT: [&str; 3] = ["郵便番号", "〒", "住所"];

pub fn detect(text: &str) -> Vec<Finding> {
    let mut found = Vec::new();

    for m in EMAIL.find_iter(text) {
        found.push(
            Finding::new(PiiKind::Email, m.start(), m.end(), Confidence::High)
                .with(Evidence::Pattern),
        );
    }

    for m in PHONE.find_iter(text).chain(PHONE_PLAIN.find_iter(text)) {
        found.push(
            Finding::new(PiiKind::PhoneNumber, m.start(), m.end(), Confidence::High)
                .with(Evidence::Pattern),
        );
    }

    for m in DATE.find_iter(text) {
        // 【重要】日付そのものは個人情報ではありません。
        // 「1980年3月15日生」の「生」があって初めて生年月日になります。
        // 文脈が無い日付を High で拾うと、契約日や納期が全部PIIになります。
        let context = context_near(text, m.start(), m.end(), &BIRTH_CONTEXT);
        let mut f = match &context {
            Some(_) => Finding::new(PiiKind::BirthDate, m.start(), m.end(), Confidence::High),
            None => Finding::new(PiiKind::BirthDate, m.start(), m.end(), Confidence::Low),
        }
        .with(Evidence::Pattern);
        if let Some(word) = context {
            f = f.with(Evidence::ContextWord(word));
        }
        found.push(f);
    }

    for m in POSTAL.find_iter(text) {
        let has_mark = m.as_str().starts_with('〒');
        let context = context_near(text, m.start(), m.end(), &POSTAL_CONTEXT);
        let mut f = if has_mark || context.is_some() {
            Finding::new(PiiKind::PostalCode, m.start(), m.end(), Confidence::High)
        } else {
            // 電話番号の一部かもしれない。言い切らない。
            Finding::new(PiiKind::PostalCode, m.start(), m.end(), Confidence::Low)
        }
        .with(Evidence::Pattern);
        if let Some(word) = context {
            f = f.with(Evidence::ContextWord(word));
        }
        found.push(f);
    }

    found
}

/// 一致の前後 24 バイトに文脈語があるか。
fn context_near(text: &str, start: usize, end: usize, words: &[&str]) -> Option<String> {
    let from = floor_boundary(text, start.saturating_sub(24));
    let to = ceil_boundary(text, (end + 24).min(text.len()));
    let window = &text[from..to];
    words
        .iter()
        .find(|w| window.contains(*w))
        .map(|w| (*w).to_string())
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_boundary(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(PiiKind, Confidence)> {
        detect(text)
            .into_iter()
            .map(|f| (f.kind, f.confidence))
            .collect()
    }

    #[test]
    fn メールアドレスを見つけること() {
        // example.com は試験用に予約されているドメイン（RFC 2606）。
        let found = detect("連絡先は taro@example.com です");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, PiiKind::Email);
        assert_eq!(found[0].confidence, Confidence::High);
    }

    #[test]
    fn 電話番号を見つけること() {
        for text in [
            "090-1234-5678",
            "03-1234-5678",
            "+81-90-1234-5678",
            "09012345678",
        ] {
            let found = detect(text);
            assert!(
                found.iter().any(|f| f.kind == PiiKind::PhoneNumber),
                "{text} を電話番号として拾えていない"
            );
        }
    }

    #[test]
    fn 生年月日は文脈が無ければ言い切らないこと() {
        // 【重要】ここを High にすると、契約日も納期も全部 PII になる。
        let bare = kinds("納期は2024年3月15日です");
        assert!(bare.contains(&(PiiKind::BirthDate, Confidence::Low)));

        let birth = kinds("1980年3月15日生まれ");
        assert!(birth.contains(&(PiiKind::BirthDate, Confidence::High)));
    }

    #[test]
    fn 郵便番号は記号か文脈で確信度が変わること() {
        assert!(kinds("〒123-4567").contains(&(PiiKind::PostalCode, Confidence::High)));
        assert!(kinds("郵便番号 123-4567").contains(&(PiiKind::PostalCode, Confidence::High)));
        // 単独の 123-4567 は電話番号の一部かもしれない。
        assert!(kinds("整理番号 123-4567").contains(&(PiiKind::PostalCode, Confidence::Low)));
    }

    #[test]
    fn 仕様書の例からすべて拾えること() {
        let text = "山田太郎さん（1980年3月15日生・090-1234-5678）の検査結果について...";
        let found = detect(text);
        assert!(found.iter().any(|f| f.kind == PiiKind::BirthDate));
        assert!(found.iter().any(|f| f.kind == PiiKind::PhoneNumber));
        // 生年月日は「生」があるので確信度が高いこと。
        let birth = found.iter().find(|f| f.kind == PiiKind::BirthDate).unwrap();
        assert_eq!(birth.confidence, Confidence::High);
    }

    #[test]
    fn 位置がバイト境界を壊さないこと() {
        let text = "担当は 山田 で、電話は090-1234-5678、以上です";
        for f in detect(text) {
            // ここで panic すればバイト位置がずれている。
            let _ = &text[f.start..f.end];
        }
    }
}
