//! 検査用数字で確かめられるもの。クレジットカード（Luhn）とマイナンバー。
//!
//! <div class="warning">
//!
//! 【重要】桁数だけで拾ってはいけません。
//!
//! 16桁の数字は注文番号にも伝票番号にもあります。検査用数字を通さずに
//! 「カード番号を検出しました」と出すと、承認画面が誤検出で埋まります。
//! 誤検出が続くと、承認する人は中身を読まずに承認するようになります。
//! それは、この仕組みが無いより危険です。
//!
//! </div>

use std::sync::LazyLock;

use regex::Regex;

use super::{Confidence, Evidence, Finding, PiiKind};

/// 区切り記号を含む数字の並び。あとで数字だけ取り出して検査する。
static DIGIT_RUN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[0-9][0-9\-\s]{9,24}[0-9]").unwrap());

/// マイナンバーらしさを補強する文脈語。
const MY_NUMBER_CONTEXT: [&str; 4] = [
    "マイナンバー",
    "個人番号",
    "my number",
    "マイナンバーカード",
];

pub fn detect(text: &str) -> Vec<Finding> {
    let mut found = Vec::new();

    for m in DIGIT_RUN.find_iter(text) {
        let digits: String = m.as_str().chars().filter(|c| c.is_ascii_digit()).collect();

        // クレジットカード: 13〜19桁で Luhn を通ること。
        if (13..=19).contains(&digits.len()) && luhn(&digits) {
            found.push(
                Finding::new(PiiKind::CreditCard, m.start(), m.end(), Confidence::High)
                    .with(Evidence::ChecksumVerified),
            );
            continue;
        }

        // マイナンバー: 12桁で検査用数字を通ること。
        if digits.len() == 12 && my_number(&digits) {
            // 【重要】検査用数字を通る12桁は、およそ11個に1個あります。
            // 電話番号や伝票番号が偶然通ることがあるので、
            // 文脈語が無ければ確信度を下げる。「見つけた」と言い切らない。
            let context = nearby_context(text, m.start(), &MY_NUMBER_CONTEXT);
            let mut f = match &context {
                Some(_) => Finding::new(PiiKind::MyNumber, m.start(), m.end(), Confidence::High),
                None => Finding::new(PiiKind::MyNumber, m.start(), m.end(), Confidence::Medium),
            }
            .with(Evidence::ChecksumVerified);
            if let Some(word) = context {
                f = f.with(Evidence::ContextWord(word));
            }
            found.push(f);
        }
    }

    found
}

/// 前後 40 バイトの範囲に文脈語があるか。
fn nearby_context(text: &str, at: usize, words: &[&str]) -> Option<String> {
    let start = floor_char_boundary(text, at.saturating_sub(40));
    let end = ceil_char_boundary(text, (at + 40).min(text.len()));
    let window = &text[start..end];
    let lower = window.to_lowercase();
    words
        .iter()
        .find(|w| lower.contains(&w.to_lowercase()))
        .map(|w| (*w).to_string())
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Luhn アルゴリズム。
///
/// 右から2桁ごとに2倍し、10以上なら各桁を足す。合計が10で割り切れれば妥当。
pub fn luhn(digits: &str) -> bool {
    if digits.len() < 2 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let sum: u32 = digits
        .bytes()
        .rev()
        .enumerate()
        .map(|(i, b)| {
            let d = u32::from(b - b'0');
            if i % 2 == 1 {
                let doubled = d * 2;
                if doubled > 9 { doubled - 9 } else { doubled }
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

/// マイナンバー（個人番号）の検査用数字。
///
/// 12桁のうち、下1桁が検査用数字。上位11桁を右から `n = 1..=11` として、
///
/// ```text
///   Q_n = n + 1  (1 <= n <= 6)
///   Q_n = n - 5  (7 <= n <= 11)
///   検査用数字 = 11 - (Σ P_n × Q_n) mod 11
///   ただし結果が 10 以上なら 0
/// ```
pub fn my_number(digits: &str) -> bool {
    if digits.len() != 12 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let bytes = digits.as_bytes();
    let check = u32::from(bytes[11] - b'0');
    let expected = my_number_check_digit(&digits[..11]);
    Some(check) == expected
}

/// 上位11桁から検査用数字を計算する。テストで架空の番号を作るのにも使う。
pub fn my_number_check_digit(first_eleven: &str) -> Option<u32> {
    if first_eleven.len() != 11 || !first_eleven.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let sum: u32 = first_eleven
        .bytes()
        .rev() // 右から n = 1, 2, ... と数える
        .enumerate()
        .map(|(i, b)| {
            let n = (i + 1) as u32;
            let q = if n <= 6 { n + 1 } else { n - 5 };
            u32::from(b - b'0') * q
        })
        .sum();
    let r = sum % 11;
    Some(if r <= 1 { 0 } else { 11 - r })
}

#[cfg(test)]
mod tests {
    use super::*;

    // 【重要】ここで使う番号はすべて架空です（仕様書9章）。
    // カードは公開されているテスト番号、マイナンバーは検査用数字から逆算した値。

    #[test]
    fn luhnを通る番号と通らない番号を分けること() {
        assert!(luhn("4242424242424242"), "公開テスト番号が通らない");
        assert!(luhn("5555555555554444"));
        assert!(!luhn("4242424242424243"), "1桁変えたら落ちるはず");
        assert!(!luhn("1234567812345678"));
    }

    #[test]
    fn luhnは数字以外を受け付けないこと() {
        assert!(!luhn("4242-4242-4242-4242"), "呼ぶ前に区切りを外すこと");
        assert!(!luhn(""));
    }

    #[test]
    fn マイナンバーの検査用数字を計算できること() {
        // 上位11桁から検査用数字を出し、それを足した12桁が検証を通ること。
        for head in ["12345678901", "00000000000", "98765432109"] {
            let check = my_number_check_digit(head).unwrap();
            let full = format!("{head}{check}");
            assert!(my_number(&full), "{full} が通らない");
        }
    }

    #[test]
    fn マイナンバーは一桁違えば落ちること() {
        let head = "12345678901";
        let check = my_number_check_digit(head).unwrap();
        let wrong = (check + 1) % 10;
        let full = format!("{head}{wrong}");
        // check と wrong が偶然一致する場合を除く
        if check != wrong {
            assert!(!my_number(&full), "{full} が通ってしまう");
        }
    }

    #[test]
    fn 区切り付きのカード番号を見つけること() {
        let text = "カード番号は 4242-4242-4242-4242 です";
        let found = detect(text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, PiiKind::CreditCard);
        assert_eq!(found[0].confidence, Confidence::High);
    }

    #[test]
    fn luhnを通らない十六桁は拾わないこと() {
        // 【重要】ここが拾われると、注文番号が全部カードとして検出される。
        let text = "注文番号 1234567812345678 の件です";
        assert!(detect(text).is_empty());
    }

    #[test]
    fn 文脈語が無いマイナンバーは確信度を下げること() {
        let head = "12345678901";
        let check = my_number_check_digit(head).unwrap();
        let number = format!("{head}{check}");

        let bare = detect(&format!("番号 {number} を登録しました"));
        assert_eq!(
            bare[0].confidence,
            Confidence::Medium,
            "言い切ってはいけない"
        );

        let with_context = detect(&format!("マイナンバー {number} を登録しました"));
        assert_eq!(with_context[0].confidence, Confidence::High);
    }

    #[test]
    fn 日本語のあいだでもバイト位置がずれないこと() {
        let text = "お客様のカードは4242424242424242、有効期限は来年です";
        let found = detect(text);
        assert_eq!(found.len(), 1);
        let slice = &text[found[0].start..found[0].end];
        assert_eq!(slice, "4242424242424242");
    }
}
