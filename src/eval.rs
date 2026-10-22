//! SemEval-2022 Task 8 English pairwise clustering-threshold evaluation.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::{
    embed::{Embedder, article_embedding_input, similarity},
    tuning,
};

#[derive(Debug, Clone)]
pub struct SimilarityPair {
    pub first: String,
    pub second: String,
    pub overall: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    pub threshold: f32,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    pub true_positive: usize,
    pub false_positive: usize,
    pub false_negative: usize,
}

#[derive(Default)]
struct LoadCounts {
    rows: usize,
    english: usize,
    malformed: usize,
    missing_articles: usize,
}

#[derive(Clone)]
struct Article {
    title: String,
    text: String,
    language: Option<String>,
}

/// Load native SemEval annotation tables alongside article JSON files, or a
/// compact custom CSV/TSV with article text in each row.
pub fn load_pairs(path: &Path) -> Result<Vec<SimilarityPair>> {
    let mut tables = Vec::new();
    let mut json_files = Vec::new();
    if path.is_file() {
        if is_json(path) {
            json_files.push(path.to_owned());
        } else if is_table(path) {
            tables.push(path.to_owned());
        } else {
            bail!(
                "expected a SemEval CSV/TSV table or a dataset directory: {}",
                path.display()
            );
        }
    } else {
        collect_files(path, &mut tables, &mut json_files)
            .with_context(|| format!("scanning evaluation directory {}", path.display()))?;
    }
    tables.sort();
    json_files.sort();
    if tables.is_empty() {
        bail!(
            "no CSV, TSV, or TXT annotation tables found under {}",
            path.display()
        );
    }
    let mut articles = HashMap::new();
    let mut json_errors = Vec::new();
    for file in json_files {
        if let Err(error) = index_article_file(&file, &mut articles) {
            json_errors.push(format!("{}: {error:#}", file.display()));
        }
    }

    let mut pairs = Vec::new();
    let mut counts = LoadCounts::default();
    let mut native_tables = 0usize;
    for file in tables {
        let content =
            fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
        let records = parse_records(&content, delimiter_for(&file, &content))
            .with_context(|| format!("parsing rows in {}", file.display()))?;
        let Some(header) = records.first() else {
            continue;
        };
        let headers = header
            .iter()
            .map(|v| canonical_header(v))
            .collect::<Vec<_>>();
        if has_native_pair_columns(&headers) {
            native_tables += 1;
            pairs.extend(
                parse_native_records(&records, &articles, &mut counts).with_context(|| {
                    format!("reading SemEval annotations from {}", file.display())
                })?,
            );
        } else {
            let inline = parse_pair_file(&content)
                .with_context(|| format!("parsing inline text pairs in {}", file.display()))?;
            pairs.extend(inline);
        }
    }
    if pairs.is_empty() {
        let json_note = if json_errors.is_empty() {
            String::new()
        } else {
            format!("; unreadable article JSON: {}", json_errors.join("; "))
        };
        bail!(
            "no usable English evaluation pairs: {} rows, {} English rows, {} malformed scores/IDs, {} missing article records{}; expected article JSON under the dataset directory",
            counts.rows,
            counts.english,
            counts.malformed,
            counts.missing_articles,
            json_note
        );
    }
    if native_tables > 0 && counts.missing_articles > 0 {
        bail!(
            "loaded {} pairs but {} English annotation rows reference missing or empty article records; ensure the complete SemEval article JSON directory is present",
            pairs.len(),
            counts.missing_articles
        );
    }
    Ok(pairs)
}

fn collect_files(root: &Path, tables: &mut Vec<PathBuf>, json: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(&path, tables, json)?;
        } else if is_json(&path) {
            json.push(path);
        } else if is_table(&path) {
            tables.push(path);
        }
    }
    Ok(())
}

fn is_json(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("json"))
}
fn is_table(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| matches!(s.to_ascii_lowercase().as_str(), "csv" | "tsv" | "txt"))
}

fn delimiter_for(path: &Path, content: &str) -> char {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("tsv") => '\t',
        Some("csv") => ',',
        _ => content
            .lines()
            .find(|l| !l.trim().is_empty())
            .map(|l| {
                if l.matches('\t').count() >= l.matches(',').count() {
                    '\t'
                } else {
                    ','
                }
            })
            .unwrap_or(','),
    }
}

/// Flexible header-based parser for compact fixtures and custom pair tables
/// that contain article text directly.
pub fn parse_pair_file(input: &str) -> Result<Vec<SimilarityPair>> {
    let delimiter = input
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| {
            if line.matches('\t').count() >= line.matches(',').count() {
                '\t'
            } else {
                ','
            }
        })
        .unwrap_or(',');
    let records = parse_records(input, delimiter)?;
    let Some(record) = records.first() else {
        bail!("empty evaluation input")
    };
    let headers = record
        .iter()
        .map(|s| canonical_header(s))
        .collect::<Vec<_>>();
    let overall_col = column(&headers, &["overall", "overall_score", "score"])
        .context("evaluation file has no overall score column")?;
    let lang_col = column(
        &headers,
        &["language", "lang", "pair_language", "pair_lang"],
    );
    let body1 = column(
        &headers,
        &[
            "text1",
            "text_a",
            "article1",
            "article_a",
            "body1",
            "body_a",
        ],
    );
    let body2 = column(
        &headers,
        &[
            "text2",
            "text_b",
            "article2",
            "article_b",
            "body2",
            "body_b",
        ],
    );
    let title1 = column(&headers, &["title1", "title_a", "headline1", "headline_a"]);
    let title2 = column(&headers, &["title2", "title_b", "headline2", "headline_b"]);
    let text1_col = body1
        .or(title1)
        .context("evaluation file has no first article text column")?;
    let text2_col = body2
        .or(title2)
        .context("evaluation file has no second article text column")?;
    let mut pairs = Vec::new();
    for row in records.iter().skip(1) {
        if lang_col.is_some_and(|index| row.get(index).is_none_or(|s| !is_english(s))) {
            continue;
        }
        let (Some(first), Some(second)) = (row.get(text1_col), row.get(text2_col)) else {
            continue;
        };
        let Some(score) = row
            .get(overall_col)
            .and_then(|s| s.trim().parse::<f32>().ok())
            .filter(|s| s.is_finite())
        else {
            continue;
        };
        let first = article_embedding_input(
            title1.and_then(|i| row.get(i)).map_or("", String::as_str),
            first,
        );
        let second = article_embedding_input(
            title2.and_then(|i| row.get(i)).map_or("", String::as_str),
            second,
        );
        if first.trim().is_empty() || second.trim().is_empty() {
            continue;
        }
        pairs.push(SimilarityPair {
            first,
            second,
            overall: score,
        });
    }
    Ok(pairs)
}

fn has_native_pair_columns(headers: &[String]) -> bool {
    column(headers, &["pair_id", "pairid", "id"]).is_some()
        && column(headers, &["overall", "overall_score", "score"]).is_some()
        && (column(headers, &["url1", "url_1", "url_a", "article_id1", "id1"]).is_some()
            || column(headers, &["url2", "url_2", "url_b", "article_id2", "id2"]).is_some())
}

fn parse_native_records(
    records: &[Vec<String>],
    articles: &HashMap<String, Article>,
    counts: &mut LoadCounts,
) -> Result<Vec<SimilarityPair>> {
    let headers = records
        .first()
        .context("missing header row")?
        .iter()
        .map(|s| canonical_header(s))
        .collect::<Vec<_>>();
    let score_col =
        column(&headers, &["overall", "overall_score", "score"]).context("no Overall column")?;
    let pair_col = column(&headers, &["pair_id", "pairid", "id"]);
    let first_col = column(&headers, &["url1", "url_1", "url_a", "article_id1", "id1"]);
    let second_col = column(&headers, &["url2", "url_2", "url_b", "article_id2", "id2"]);
    let lang_col = column(
        &headers,
        &["pair_lang", "pair_language", "language", "lang"],
    );
    let first_lang_col = column(&headers, &["lang1", "language1", "language_a"]);
    let second_lang_col = column(&headers, &["lang2", "language2", "language_b"]);
    let mut pairs = Vec::new();
    for row in records
        .iter()
        .skip(1)
        .filter(|r| !r.iter().all(|v| v.trim().is_empty()))
    {
        counts.rows += 1;
        let Some(score) = row
            .get(score_col)
            .and_then(|s| s.trim().parse::<f32>().ok())
            .filter(|s| s.is_finite())
        else {
            counts.malformed += 1;
            continue;
        };
        let pair_id = pair_col.and_then(|i| row.get(i)).map(String::as_str);
        let direct_refs = pair_references(
            None,
            first_col.and_then(|i| row.get(i)).map(String::as_str),
            second_col.and_then(|i| row.get(i)).map(String::as_str),
        );
        let pair_refs = pair_references(pair_id, None, None);
        let refs = pair_refs.or(direct_refs);
        let Some((first_ref, second_ref)) = refs else {
            counts.malformed += 1;
            continue;
        };
        let row_lang = lang_col.and_then(|i| row.get(i)).map(String::as_str);
        if row_lang.is_some_and(|lang| !is_english_pair(lang)) {
            continue;
        }
        if first_lang_col
            .and_then(|i| row.get(i))
            .is_some_and(|lang| !is_english(lang))
            || second_lang_col
                .and_then(|i| row.get(i))
                .is_some_and(|lang| !is_english(lang))
        {
            continue;
        }
        let mut first = find_article(articles, first_ref);
        let mut second = find_article(articles, second_ref);
        if (first.is_none() || second.is_none())
            && direct_refs.is_some()
            && pair_refs.is_some()
            && let Some((a, b)) = direct_refs
        {
            first = find_article(articles, a);
            second = find_article(articles, b);
        }
        if first
            .as_ref()
            .and_then(|a| a.language.as_deref())
            .is_some_and(|l| !is_english(l))
            || second
                .as_ref()
                .and_then(|a| a.language.as_deref())
                .is_some_and(|l| !is_english(l))
        {
            continue;
        }
        counts.english += 1;
        let (Some(first), Some(second)) = (first, second) else {
            counts.missing_articles += 1;
            continue;
        };
        let first_text = article_embedding_input(&first.title, &first.text);
        let second_text = article_embedding_input(&second.title, &second.text);
        if first_text.trim().is_empty() || second_text.trim().is_empty() {
            counts.missing_articles += 1;
            continue;
        }
        pairs.push(SimilarityPair {
            first: first_text,
            second: second_text,
            overall: score,
        });
    }
    Ok(pairs)
}

fn pair_references<'a>(
    pair_id: Option<&'a str>,
    first: Option<&'a str>,
    second: Option<&'a str>,
) -> Option<(&'a str, &'a str)> {
    if let (Some(first), Some(second)) = (
        first.filter(|s| !s.trim().is_empty()),
        second.filter(|s| !s.trim().is_empty()),
    ) {
        return Some((first.trim(), second.trim()));
    }
    let id = pair_id?.trim();
    // SemEval pair IDs use the two article identifiers separated by an underscore.
    let (left, right) = id.split_once('_')?;
    (!left.is_empty() && !right.is_empty()).then_some((left, right))
}

fn find_article<'a>(
    articles: &'a HashMap<String, Article>,
    reference: &str,
) -> Option<&'a Article> {
    let reference = reference.trim();
    let basename = reference
        .rsplit('/')
        .next()
        .unwrap_or(reference)
        .rsplit('\\')
        .next()
        .unwrap_or(reference);
    let stem = basename.strip_suffix(".json").unwrap_or(basename);
    [reference, basename, stem]
        .iter()
        .find_map(|key| articles.get(&normalize_id(key)))
}

fn index_article_file(path: &Path, index: &mut HashMap<String, Article>) -> Result<()> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("reading article JSON {}", path.display()))?;
    let value: Value = serde_json::from_str(&content)
        .with_context(|| format!("decoding article JSON {}", path.display()))?;
    let fallback = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    index_json_articles(&value, fallback, index);
    Ok(())
}

fn index_json_articles(value: &Value, fallback_id: &str, index: &mut HashMap<String, Article>) {
    match value {
        Value::Array(items) => {
            for item in items {
                index_json_articles(item, fallback_id, index);
            }
        }
        Value::Object(object) => {
            let id = string_field(object, &["id", "article_id", "articleid", "url_id"])
                .map(str::to_owned)
                .or_else(|| scalar_field(object, &["id", "article_id", "articleid", "url_id"]))
                .unwrap_or_else(|| fallback_id.to_owned());
            let title = string_field(object, &["title", "headline"]).unwrap_or_default();
            let text = string_field(object, &["text", "body", "content", "article_text"])
                .unwrap_or_default();
            if !id.trim().is_empty() && (!title.trim().is_empty() || !text.trim().is_empty()) {
                let language = string_field(object, &["language", "lang", "article_language"]);
                let article = Article {
                    title: title.to_owned(),
                    text: text.to_owned(),
                    language: language.map(str::to_owned),
                };
                index.insert(normalize_id(&id), article);
            }
            for (key, nested) in object {
                if nested.is_object() || nested.is_array() {
                    index_json_articles(nested, key, index);
                }
            }
        }
        _ => (),
    }
}

fn scalar_field(object: &serde_json::Map<String, Value>, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        object
            .iter()
            .find(|(key, _)| canonical_header(key) == *name)
            .and_then(|(_, value)| match value {
                Value::Number(number) => Some(number.to_string()),
                _ => None,
            })
    })
}

fn string_field<'a>(object: &'a serde_json::Map<String, Value>, names: &[&str]) -> Option<&'a str> {
    names.iter().find_map(|name| {
        object
            .iter()
            .find(|(key, _)| canonical_header(key) == *name)
            .and_then(|(_, value)| value.as_str())
    })
}

fn normalize_id(value: &str) -> String {
    value.trim().trim_matches('\u{feff}').to_ascii_lowercase()
}

fn column(headers: &[String], names: &[&str]) -> Option<usize> {
    headers
        .iter()
        .position(|header| names.iter().any(|name| header == name))
}

fn is_english(language: &str) -> bool {
    let language = language.trim().to_ascii_lowercase().replace('_', "-");
    matches!(language.as_str(), "en" | "eng" | "english") || language.starts_with("en-")
}

fn is_english_pair(language: &str) -> bool {
    let language = language.trim().to_ascii_lowercase();
    if language.is_empty() {
        return true;
    }
    if language.contains('-') || language.contains('_') {
        language
            .split(['-', '_'])
            .all(|part| part == "en" || part == "eng" || part == "english")
    } else {
        is_english(&language)
    }
}

fn canonical_header(value: &str) -> String {
    value
        .trim()
        .trim_matches('\u{feff}')
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
}

/// Parse CSV/TSV records including quoted delimiters, doubled quotes and
/// embedded newlines. The input is UTF-8 and the delimiter is selected by the
/// file extension or header row.
fn parse_records(input: &str, delimiter: char) -> Result<Vec<Vec<String>>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut chars = input.chars().peekable();
    let mut quoted = false;
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            c if c == delimiter && !quoted => record.push(std::mem::take(&mut field)),
            '\n' if !quoted => {
                record.push(std::mem::take(&mut field));
                if record.iter().any(|part: &String| !part.is_empty()) {
                    records.push(std::mem::take(&mut record));
                }
            }
            '\r' if !quoted && chars.peek() == Some(&'\n') => (),
            _ => field.push(ch),
        }
    }
    if quoted {
        bail!("unterminated quoted CSV field")
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    Ok(records)
}

/// Evaluate real model embeddings for the specified thresholds, 0.70 through 0.95.
pub fn evaluate(pairs: &[SimilarityPair], embedder: &Embedder) -> Result<Vec<Metrics>> {
    let mut similarities = Vec::with_capacity(pairs.len());
    let mut same = Vec::with_capacity(pairs.len());
    for pair in pairs {
        let first = embedder.embed(&pair.first)?;
        let second = embedder.embed(&pair.second)?;
        similarities
            .push(similarity(&first, &second).context("embedding dimensions did not match")?);
        same.push(pair.overall <= 1.5);
    }
    let mut result = Vec::new();
    for step in 70..=95 {
        let threshold = step as f32 / 100.0;
        let mut tp = 0;
        let mut fp = 0;
        let mut fn_ = 0;
        for (score, is_same) in similarities.iter().zip(&same) {
            match (*score >= threshold, *is_same) {
                (true, true) => tp += 1,
                (true, false) => fp += 1,
                (false, true) => fn_ += 1,
                (false, false) => (),
            }
        }
        let precision = ratio(tp, tp + fp);
        let recall = ratio(tp, tp + fn_);
        let f1 = if precision + recall == 0.0 {
            0.0
        } else {
            2.0 * precision * recall / (precision + recall)
        };
        result.push(Metrics {
            threshold,
            precision,
            recall,
            f1,
            true_positive: tp,
            false_positive: fp,
            false_negative: fn_,
        });
    }
    Ok(result)
}

pub fn print_report(metrics: &[Metrics]) {
    println!("calibration={}", tuning::JOIN_THRESHOLD_CALIBRATION);
    println!(
        "model=potion-base-8M threshold={:.2}..{:.2} step=0.01; join threshold default={:.2}",
        0.70,
        0.95,
        tuning::JOIN_THRESHOLD
    );
    println!("threshold\tprecision\trecall\tf1\ttp\tfp\tfn");
    for m in metrics {
        println!(
            "{:.2}\t{:.4}\t{:.4}\t{:.4}\t{}\t{}\t{}",
            m.threshold,
            m.precision,
            m.recall,
            m.f1,
            m.true_positive,
            m.false_positive,
            m.false_negative
        );
    }
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("wagwan-eval-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).expect("create temporary evaluation fixture directory");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn loads_native_semeval_pairs_and_filters_non_english_rows() {
        let dir = TempDir::new();
        fs::write(
            dir.0.join("pairs.csv"),
            "pairID,URL1,URL2,Overall,Pair_Lang\nA_B,A.json,B.json,1.2,eng\nC_D,C.json,D.json,4.5,fr\n",
        ).unwrap();
        fs::write(
            dir.0.join("articles.json"),
            r#"[
              {"article_id":"A","title":"Synthetic shared event","text":"English first article body.","language":"en"},
              {"article_id":"B","title":"Synthetic shared event update","text":"English second article body.","language":"en"},
              {"article_id":"C","title":"French headline","text":"French content","language":"fr"},
              {"article_id":"D","title":"French headline update","text":"French content","language":"fr"}
            ]"#,
        ).unwrap();

        let pairs = load_pairs(&dir.0).unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].overall, 1.2);
        assert!(pairs[0].first.starts_with("Synthetic shared event\n"));
        assert!(pairs[0].second.contains("English second article body."));
    }

    #[test]
    fn compact_inline_table_uses_english_and_article_embedding_input() {
        let pairs = parse_pair_file(
            "language,title1,text1,title2,text2,overall\nen-US,First,First body,Second,Second body,1.0\nfr,Premier,Texte,Deuxieme,Texte,1.0\n",
        ).unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].first, "First\nFirst body");
        assert_eq!(pairs[0].second, "Second\nSecond body");
    }
}
