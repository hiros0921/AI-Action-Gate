//! 氏名の検出。
//!
//! <div class="warning">
//!
//! 【重要】日本語の氏名は、機械的には確定できません。
//!
//! 「田中」は姓でもあり地名でもあります。「新井」は姓でもあり普通名詞の並びでもあります。
//! どれだけ辞書を厚くしても、この曖昧さは残ります。
//!
//! そこで、**見つかったかどうか**ではなく、**何を根拠に見つけたか**を残します。
//! 根拠の強さがそのまま確信度になります。
//!
//! | 確信度 | 根拠 | 例 |
//! |---|---|---|
//! | High | 敬称、またはラベル | `山田太郎さん` / `氏名：山田太郎` |
//! | Medium | 姓の辞書 ＋ 直後が名らしい | `山田太郎の検査結果` |
//! | Low | 形が似ているだけ | `タナカ・タロウ` |
//!
//! Low を捨てないのは、**捨てた瞬間に「無かった」ことになる**からです。
//! 承認画面には出し、承認者が「これは人名ではない」と外せる形にします。
//!
//! </div>

use std::sync::LazyLock;

use regex::Regex;

use super::{Confidence, Evidence, Finding, PiiKind};

/// 敬称つき。`(名前)(敬称)` の形。
static HONORIFIC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([\p{Han}]{2,4}|[\p{Katakana}ー]{2,8})(さん|様|氏|殿|さま)").unwrap()
});

/// ラベルつき。`氏名：山田太郎` の形。
static LABELED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(氏名|名前|担当者|担当|宛先|申請者|患者|利用者|お客様名)\s*[:：]\s*([\p{Han}]{2,5}|[\p{Katakana}ー\s]{2,10})",
    )
    .unwrap()
});

/// カタカナの姓名。`タナカ・タロウ` `ヤマダ タロウ`。
static KATAKANA_PAIR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\p{Katakana}ー]{2,6}[・\s][\p{Katakana}ー]{2,6}").unwrap());

/// 漢字の並び。姓の辞書と突き合わせる。
static KANJI_RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\p{Han}]{2,5}").unwrap());

/// 敬称が付いても人名ではない語。
///
/// 【重要】これが無いと「お客様」から「お客」を、「皆様」から「皆」を
/// 氏名として拾います。承認画面が誤検出で埋まると、承認する人は中身を読まなくなります。
const NOT_NAMES: &[&str] = &[
    "お客",
    "皆",
    "神",
    "奥",
    "御中",
    "各位",
    "担当",
    "御社",
    "貴社",
    "弊社",
    "当社",
    "先方",
    "本人",
    "同席",
    "関係",
    "利用者",
    "責任",
    "代表",
    "取締",
    "部長",
    "課長",
    "社長",
];

/// 姓の辞書。公開されている頻度上位の姓。
///
/// 【重要】これは**特定の個人を指す情報ではありません**（仕様書9章）。
/// 姓そのものは公開情報で、ここに実在の人物の氏名は入っていません。
const SURNAMES: &[&str] = &[
    "佐藤",
    "鈴木",
    "高橋",
    "田中",
    "伊藤",
    "渡辺",
    "山本",
    "中村",
    "小林",
    "加藤",
    "吉田",
    "山田",
    "佐々木",
    "山口",
    "松本",
    "井上",
    "木村",
    "林",
    "斎藤",
    "清水",
    "山崎",
    "阿部",
    "森",
    "池田",
    "橋本",
    "石川",
    "山下",
    "小川",
    "石井",
    "長谷川",
    "後藤",
    "岡田",
    "近藤",
    "前田",
    "藤田",
    "遠藤",
    "青木",
    "坂本",
    "斉藤",
    "福田",
    "太田",
    "西村",
    "藤井",
    "金子",
    "岡本",
    "藤原",
    "中島",
    "中野",
    "原田",
    "小野",
    "田村",
    "竹内",
    "中川",
    "和田",
    "石田",
    "森田",
    "上田",
    "原",
    "内田",
    "柴田",
    "酒井",
    "宮崎",
    "横山",
    "高木",
    "安藤",
    "宮本",
    "大野",
    "小島",
    "谷口",
    "工藤",
    "今井",
    "高田",
    "増田",
    "丸山",
    "杉山",
    "村上",
    "大塚",
    "小山",
    "菅原",
    "武田",
    "新井",
    "野口",
    "松田",
    "千葉",
    "岩崎",
    "桜井",
    "木下",
    "野村",
    "松尾",
    "菊地",
    "川口",
    "野崎",
    "菊池",
    "島田",
    "渡部",
    "早川",
    "永井",
    "松岡",
    "本田",
    "水野",
    "秋山",
    "田口",
    "大西",
    "森本",
    "白石",
    "堀",
    "浜田",
    "市川",
    "山内",
    "岡崎",
    "吉川",
    "中山",
    "西田",
    "服部",
    "上野",
    "望月",
    "須藤",
    "内藤",
    "松浦",
    "大久保",
];

/// 敬称が付いていても氏名ではない語か。
///
/// 【重要】語尾も見る。実測で「担当者様」から「担当者」を氏名として拾っていた。
/// 一覧に「担当」は入れてあったが「担当者」は入れ忘れていた。
/// この形の抜けは、語を足していく方式では必ずまた起きる。
/// 「〜者」「〜員」「〜方」は役割を指す語なので、まとめて外す。
fn is_role_word(word: &str) -> bool {
    NOT_NAMES.contains(&word)
        || word.ends_with('者')
        || word.ends_with('員')
        || word.ends_with('方')
        || word.ends_with('位')
}

pub fn detect(text: &str) -> Vec<Finding> {
    let mut found = Vec::new();

    // ① 敬称。いちばん強い根拠。
    for caps in HONORIFIC.captures_iter(text) {
        let name = caps.get(1).unwrap();
        let honorific = caps.get(2).unwrap().as_str();
        if is_role_word(name.as_str()) {
            continue;
        }
        found.push(
            Finding::new(
                PiiKind::PersonName,
                name.start(),
                name.end(),
                Confidence::High,
            )
            .with(Evidence::Honorific(honorific.to_string())),
        );
    }

    // ② ラベル。「氏名：」の直後は氏名とみてよい。
    for caps in LABELED.captures_iter(text) {
        let label = caps.get(1).unwrap().as_str();
        let name = caps.get(2).unwrap();
        let trimmed = name.as_str().trim_end();
        if trimmed.is_empty() {
            continue;
        }
        found.push(
            Finding::new(
                PiiKind::PersonName,
                name.start(),
                name.start() + trimmed.len(),
                Confidence::High,
            )
            .with(Evidence::Label(label.to_string())),
        );
    }

    // ③ 姓の辞書。
    for m in KANJI_RUN.find_iter(text) {
        let run = m.as_str();
        let Some(surname) = SURNAMES
            .iter()
            .filter(|s| run.starts_with(**s))
            // 「佐々木」と「佐藤」のように前方一致が重なるので、長いほうを採る。
            .max_by_key(|s| s.len())
        else {
            continue;
        };
        let rest = run.len() - surname.len();
        if rest == 0 {
            // 姓だけ。地名かもしれない（山田町・中村区）。言い切らない。
            found.push(
                Finding::new(PiiKind::PersonName, m.start(), m.end(), Confidence::Low)
                    .with(Evidence::SurnameDictionary),
            );
        } else {
            // 姓 ＋ 1〜2文字。名前の形をしている。
            found.push(
                Finding::new(PiiKind::PersonName, m.start(), m.end(), Confidence::Medium)
                    .with(Evidence::SurnameDictionary),
            );
        }
    }

    // ④ カタカナの姓名。形だけ。
    for m in KATAKANA_PAIR.find_iter(text) {
        found.push(
            Finding::new(PiiKind::PersonName, m.start(), m.end(), Confidence::Low)
                .with(Evidence::Pattern),
        );
    }

    found
}

#[cfg(test)]
mod tests {
    use super::*;

    // 【重要】ここに出てくる氏名はすべて架空です（仕様書9章）。

    fn best(text: &str) -> Option<Finding> {
        let mut found = detect(text);
        found.sort_by_key(|f| std::cmp::Reverse(f.confidence));
        found.into_iter().next()
    }

    #[test]
    fn 敬称が付いていれば言い切ること() {
        let f = best("山田太郎さんの検査結果").unwrap();
        assert_eq!(f.confidence, Confidence::High);
        assert!(matches!(f.evidence[0], Evidence::Honorific(_)));
    }

    #[test]
    fn ラベルの直後も言い切ること() {
        let f = best("氏名：山田太郎").unwrap();
        assert_eq!(f.confidence, Confidence::High);
        assert!(matches!(f.evidence[0], Evidence::Label(_)));
    }

    #[test]
    fn 姓と名の並びは辞書で拾うこと() {
        let f = best("山田太郎の検査結果について").unwrap();
        assert_eq!(f.confidence, Confidence::Medium);
    }

    #[test]
    fn 姓だけなら言い切らないこと() {
        // 「山田」は地名にもある。ここを Medium にすると誤検出が増える。
        let found = detect("山田工業に発注しました");
        let name = found.iter().find(|f| f.start == 0).unwrap();
        assert!(
            name.confidence <= Confidence::Medium,
            "姓だけで言い切っている"
        );
    }

    #[test]
    fn 敬称が付いても人名でない語を拾わないこと() {
        // 【重要】これが無いと「お客様」から「お客」を氏名として拾う。
        for text in [
            "お客様各位",
            "皆様におかれましては",
            "担当者様", // 実測で拾ってしまった形
            "関係者各位",
            "配送業者様",
        ] {
            let found = detect(text);
            assert!(
                !found.iter().any(|f| f.confidence == Confidence::High),
                "{text} を氏名として言い切っている: {found:?}"
            );
        }
    }

    #[test]
    fn カタカナの姓名は形だけとして扱うこと() {
        let f = best("タナカ・タロウ").unwrap();
        assert_eq!(f.confidence, Confidence::Low);
    }

    #[test]
    fn 人名を含まない文で言い切らないこと() {
        // 誤検出そのものは避けられないが、High が出てはいけない。
        for text in [
            "在庫の確認をお願いします",
            "会議室の予約を変更しました",
            "請求書を送付いたします",
        ] {
            let found = detect(text);
            assert!(
                !found.iter().any(|f| f.confidence == Confidence::High),
                "{text} で氏名を言い切っている: {found:?}"
            );
        }
    }

    #[test]
    fn 位置がバイト境界を壊さないこと() {
        let text = "本日、山田太郎さんと打ち合わせを行いました";
        for f in detect(text) {
            let _ = &text[f.start..f.end];
        }
    }
}
