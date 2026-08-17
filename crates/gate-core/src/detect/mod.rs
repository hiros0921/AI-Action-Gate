//! PII の検出。
//!
//! ここは「見つける」だけを担当する。点は付けないし、隠しもしない。
//! 変える理由が違うものを、同じ場所に置かないため（配点は [`crate::score`]、
//! 伏せ字は [`crate::mask`]）。

pub mod checksum;
pub mod name;
pub mod pattern;

use serde::{Deserialize, Serialize};

/// 見つかったものの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiiKind {
    Email,
    PhoneNumber,
    BirthDate,
    PostalCode,
    CreditCard,
    MyNumber,
    PersonName,
}

impl PiiKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Email => "メールアドレス",
            Self::PhoneNumber => "電話番号",
            Self::BirthDate => "生年月日",
            Self::PostalCode => "郵便番号",
            Self::CreditCard => "クレジットカード番号",
            Self::MyNumber => "マイナンバー",
            Self::PersonName => "氏名",
        }
    }
}

/// どれくらい確からしいか。
///
/// <div class="warning">
///
/// 【重要】数値ではなく段階にしてあります。
///
/// `0.73` のような数字を持たせると、その 0.73 に根拠があるように見えます。
/// 実際にあるのは「敬称が付いていた」「辞書に載っていた」「形が似ていた」という
/// **証拠の種類**だけです。段階にしておけば、あとから
/// 「なぜ Medium なのか」を [`Evidence`] で説明できます。
///
/// </div>
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// 形が似ているだけ。誤検出しうる。
    Low,
    /// 辞書などの裏付けがある。
    Medium,
    /// 敬称・ラベル・検査数字など、決め手がある。
    High,
}

/// 何を根拠に見つけたか。確信度の理由を、あとから読める形で残す。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    /// 正規表現に一致した。
    Pattern,
    /// 検査用数字を通った（Luhn・マイナンバー）。
    ChecksumVerified,
    /// 敬称が付いていた（さん・様・氏・殿）。
    Honorific(String),
    /// 直前にラベルがあった（氏名: など）。
    Label(String),
    /// 姓の辞書に載っていた。
    SurnameDictionary,
    /// 近くに文脈語があった（個人番号・生年月日 など）。
    ContextWord(String),
}

impl Evidence {
    pub fn describe(&self) -> String {
        match self {
            Self::Pattern => "形が一致".to_string(),
            Self::ChecksumVerified => "検査用数字が一致".to_string(),
            Self::Honorific(h) => format!("敬称「{h}」が付いている"),
            Self::Label(l) => format!("直前に「{l}」がある"),
            Self::SurnameDictionary => "姓の辞書に一致".to_string(),
            Self::ContextWord(w) => format!("近くに「{w}」がある"),
        }
    }
}

/// 見つかった 1 件。
///
/// `start` / `end` は**バイト位置**（`&str` のスライスにそのまま使える）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub kind: PiiKind,
    pub start: usize,
    pub end: usize,
    pub confidence: Confidence,
    pub evidence: Vec<Evidence>,
}

impl Finding {
    pub fn new(kind: PiiKind, start: usize, end: usize, confidence: Confidence) -> Self {
        Self {
            kind,
            start,
            end,
            confidence,
            evidence: Vec::new(),
        }
    }

    pub fn with(mut self, evidence: Evidence) -> Self {
        self.evidence.push(evidence);
        self
    }

    fn overlaps(&self, other: &Finding) -> bool {
        self.start < other.end && other.start < self.end
    }

    /// 重なったときの強さ。強いほうを残す。
    fn strength(&self) -> (u8, usize) {
        let by_evidence = if self
            .evidence
            .iter()
            .any(|e| matches!(e, Evidence::ChecksumVerified))
        {
            3
        } else {
            match self.confidence {
                Confidence::High => 2,
                Confidence::Medium => 1,
                Confidence::Low => 0,
            }
        };
        // 同じ強さなら、長く取れているほうを残す。
        (by_evidence, self.end - self.start)
    }
}

/// なぜ走査しなかったか。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// 本文が上限を超えていた。
    PayloadTooLarge { bytes: usize, limit: usize },
    /// テキストとして読めなかった。
    UnsupportedEncoding,
    /// 姓の辞書が使えなかった。
    DictionaryUnavailable,
}

impl SkipReason {
    pub fn message(&self) -> String {
        match self {
            Self::PayloadTooLarge { bytes, limit } => {
                format!("本文が大きすぎて走査していません（{bytes} バイト。上限 {limit}）")
            }
            Self::UnsupportedEncoding => "テキストとして読めないため走査していません".to_string(),
            Self::DictionaryUnavailable => "姓の辞書が使えないため走査していません".to_string(),
        }
    }
}

/// 走査の結果。
///
/// <div class="warning">
///
/// 【重要】「見つからなかった」と「見ていない」を、同じ型で表しません（仕様書5章）。
///
/// `Scanned { findings: [] }` は**走査した上で0件**。
/// `NotScanned { .. }` は**走査していない**ので、0件だとは言えません。
///
/// これを `Vec<Finding>` ひとつで表すと、両方とも空配列になります。
/// そうなると、いちばん危ない「読めなかった本文」が、いちばん安全な
/// 「PIIの無い本文」と同じ扱いで自動承認されます。
///
/// </div>
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scan {
    Scanned { findings: Vec<Finding> },
    NotScanned { reason: SkipReason },
}

impl Scan {
    /// 走査したうえで見つかったもの。走査していない場合は空。
    ///
    /// **この関数の戻り値だけで「PIIは無かった」と判断してはいけません。**
    /// 判断には [`Scan::is_conclusive`] を併せて見ること。
    pub fn findings(&self) -> &[Finding] {
        match self {
            Self::Scanned { findings } => findings,
            Self::NotScanned { .. } => &[],
        }
    }

    /// 走査を終えているか。false なら「PIIが無い」とは言えない。
    pub fn is_conclusive(&self) -> bool {
        matches!(self, Self::Scanned { .. })
    }

    /// 種別ごとの件数。監査ログにはこれだけを残す（平文は残さない）。
    pub fn summary(&self) -> Vec<KindCount> {
        let mut counts: Vec<KindCount> = Vec::new();
        for f in self.findings() {
            match counts.iter_mut().find(|c| c.kind == f.kind) {
                Some(c) => {
                    c.count += 1;
                    c.lowest_confidence = c.lowest_confidence.min(f.confidence);
                }
                None => counts.push(KindCount {
                    kind: f.kind,
                    count: 1,
                    lowest_confidence: f.confidence,
                }),
            }
        }
        counts.sort_by_key(|c| c.kind);
        counts
    }
}

/// 監査ログと応答に載せる要約。**平文は入っていない。**
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindCount {
    pub kind: PiiKind,
    pub count: usize,
    /// その種別でいちばん低かった確信度。低いものが混ざっていることを隠さない。
    pub lowest_confidence: Confidence,
}

/// 走査の設定。
#[derive(Debug, Clone)]
pub struct DetectConfig {
    /// これを超える本文は走査しない。
    pub payload_limit: usize,
    /// 氏名の検出を行うか。第2段階では常に true。
    pub detect_names: bool,
}

impl Default for DetectConfig {
    fn default() -> Self {
        Self {
            // 【重要】この値は暫定です。正規表現を全文に走らせるので、
            // 上限が無いと処理時間が読めなくなります（仕様書6章の「処理量が読めない」）。
            // 実測して決め直す対象。
            payload_limit: 64 * 1024,
            detect_names: true,
        }
    }
}

/// 本文を走査する。
///
/// ここが [`crate::score`] より前に来る唯一の入口。
pub fn scan(text: &str, cfg: &DetectConfig) -> Scan {
    if text.len() > cfg.payload_limit {
        return Scan::NotScanned {
            reason: SkipReason::PayloadTooLarge {
                bytes: text.len(),
                limit: cfg.payload_limit,
            },
        };
    }

    let mut findings = Vec::new();
    findings.extend(checksum::detect(text));
    findings.extend(pattern::detect(text));
    if cfg.detect_names {
        findings.extend(name::detect(text));
    }

    Scan::Scanned {
        findings: resolve_overlaps(findings),
    }
}

/// 重なった検出を1つに絞る。
///
/// 【重要】ここが無いと、同じ12桁が「マイナンバー」と「電話番号」の
/// 両方で数えられ、件数が水増しされます。件数はスコアに効くので、
/// 重なりを放置すると点数が実態より高く出ます。
fn resolve_overlaps(mut findings: Vec<Finding>) -> Vec<Finding> {
    // 強い順に見て、すでに採用したものと重なるなら捨てる。
    findings.sort_by(|a, b| b.strength().cmp(&a.strength()).then(a.start.cmp(&b.start)));

    let mut kept: Vec<Finding> = Vec::new();
    for f in findings {
        if !kept.iter().any(|k| k.overlaps(&f)) {
            kept.push(f);
        }
    }
    kept.sort_by_key(|f| f.start);
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 走査していないことと零件を区別すること() {
        // 【重要】ここが崩れると、読めなかった本文が自動承認される。
        let empty = Scan::Scanned { findings: vec![] };
        let skipped = Scan::NotScanned {
            reason: SkipReason::UnsupportedEncoding,
        };

        assert!(empty.findings().is_empty());
        assert!(skipped.findings().is_empty()); // 見た目は同じ
        assert!(empty.is_conclusive());
        assert!(!skipped.is_conclusive()); // 型では違う
    }

    #[test]
    fn 上限を超えたら走査しないこと() {
        let cfg = DetectConfig {
            payload_limit: 10,
            ..Default::default()
        };
        let scan = scan("これは十バイトを超える本文です", &cfg);
        assert!(!scan.is_conclusive());
        assert!(matches!(
            scan,
            Scan::NotScanned {
                reason: SkipReason::PayloadTooLarge { .. }
            }
        ));
    }

    #[test]
    fn 重なった検出を二重に数えないこと() {
        let a = Finding::new(PiiKind::MyNumber, 0, 12, Confidence::High)
            .with(Evidence::ChecksumVerified);
        let b = Finding::new(PiiKind::PhoneNumber, 0, 11, Confidence::Low);
        let kept = resolve_overlaps(vec![b, a]);
        assert_eq!(kept.len(), 1);
        assert_eq!(
            kept[0].kind,
            PiiKind::MyNumber,
            "検査数字を通ったほうを残す"
        );
    }

    #[test]
    fn 要約に平文が入らないこと() {
        let scan = scan("連絡先は taro@example.com です", &DetectConfig::default());
        let summary = scan.summary();
        let json = serde_json::to_string(&summary).unwrap();
        assert!(!json.contains("taro"), "要約に平文が混ざっている: {json}");
        assert!(json.contains("email"));
    }

    #[test]
    fn 要約はいちばん低い確信度を残すこと() {
        // 高いものと低いものが混ざったとき、低いほうを隠さない。
        let findings = vec![
            Finding::new(PiiKind::PersonName, 0, 3, Confidence::High),
            Finding::new(PiiKind::PersonName, 10, 13, Confidence::Low),
        ];
        let scan = Scan::Scanned { findings };
        assert_eq!(scan.summary()[0].lowest_confidence, Confidence::Low);
    }
}
