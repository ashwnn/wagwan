//! Article text extraction, cleanup, and deterministic sentence segmentation.

use anyhow::{Context, Result};
use dom_smoothie::Readability;
use reqwest::Client;

use crate::fetch::{
    FetchLimits, REQUEST_TIMEOUT, can_append_chunk, get_limited, validate_content_length,
};
use crate::tuning;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextSource {
    Page,
    Feed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedArticle {
    pub text: String,
    pub text_source: TextSource,
    pub sentences: Vec<String>,
}

/// Download an article page, try Readability extraction, then use the feed body
/// or summary when the page is unavailable or too short.
pub async fn extract_article(
    client: &Client,
    limits: &FetchLimits,
    url: &str,
    feed_content: Option<&str>,
) -> Result<ExtractedArticle> {
    let page = async {
        let download = async {
            let (mut response, permit) = get_limited(client, limits, url, None, None)
                .await
                .context("fetching article")?;
            anyhow::ensure!(
                response.status().is_success(),
                "article returned HTTP {}",
                response.status()
            );
            validate_content_length(response.content_length())
                .context("article exceeds 5 MB limit")?;
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                anyhow::ensure!(
                    can_append_chunk(bytes.len(), chunk.len()),
                    "article exceeds 5 MB limit"
                );
                bytes.extend_from_slice(&chunk);
            }
            drop(permit);
            Ok::<Vec<u8>, anyhow::Error>(bytes)
        };
        let bytes = tokio::time::timeout(REQUEST_TIMEOUT, download)
            .await
            .context("article download exceeded the 20 second timeout")??;
        let html = String::from_utf8_lossy(&bytes).into_owned();
        let document_url = url.to_owned();
        tokio::task::spawn_blocking(move || extract_page_html(&html, &document_url))
            .await
            .context("article extraction worker failed")?
    }
    .await;
    let (text, text_source) = choose_article_text(page, feed_content);
    let sentences = split_sentences(&text);
    Ok(ExtractedArticle {
        text,
        text_source,
        sentences,
    })
}

fn extract_page_html(html: &str, url: &str) -> Result<String> {
    let mut parser =
        Readability::new(html, Some(url), None).context("initializing article extractor")?;
    let article = parser.parse().context("extracting article text")?;
    Ok(normalize_text(&article.text_content))
}

fn choose_article_text(page: Result<String>, feed_content: Option<&str>) -> (String, TextSource) {
    match page {
        Ok(text) if text.chars().count() >= tuning::ARTICLE_FALLBACK_MIN_CHARS => {
            (text, TextSource::Page)
        }
        _ => (
            normalize_text(&strip_html(feed_content.unwrap_or_default())),
            TextSource::Feed,
        ),
    }
}

/// Collapse whitespace and omit short lines that are common page boilerplate.
pub fn normalize_text(input: &str) -> String {
    input
        .lines()
        .map(str::trim)
        .filter(|line| {
            !(line.chars().count() < tuning::BOILERPLATE_MAX_CHARS && is_boilerplate(line))
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_boilerplate(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "advertisement",
        "share this",
        "sign up",
        "subscribe",
        "cookie",
        "follow us",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// Minimal HTML text extraction used for feed-provided content and summaries.
/// Script/style bodies are skipped; common named and numeric entities are decoded.
pub fn strip_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut inside = false;
    let mut tag = String::new();
    let mut skip = false;
    let mut skip_tag = String::new();
    for ch in input.chars() {
        if ch == '<' {
            inside = true;
            tag.clear();
            continue;
        }
        if ch == '>' && inside {
            inside = false;
            let t = tag
                .trim()
                .trim_start_matches('/')
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if tag.trim_start().starts_with('/') && t == skip_tag {
                skip = false;
                skip_tag.clear();
            } else if !tag.trim_start().starts_with('/') && (t == "script" || t == "style") {
                skip = true;
                skip_tag = t;
            } else if ["p", "div", "br", "li", "h1", "h2", "h3", "blockquote"].contains(&t.as_str())
            {
                out.push(' ');
            }
            continue;
        }
        if inside {
            tag.push(ch);
        } else if !skip {
            out.push(ch);
        }
    }
    decode_entities(&out)
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '&' {
            out.push(ch);
            continue;
        }
        let mut entity = String::new();
        while let Some(&c) = chars.peek() {
            if c == ';' || entity.len() > 12 {
                break;
            }
            entity.push(c);
            chars.next();
        }
        if chars.peek() == Some(&';') {
            chars.next();
            let decoded = match entity.as_str() {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                    u32::from_str_radix(&entity[2..], 16)
                        .ok()
                        .and_then(char::from_u32)
                }
                _ if entity.starts_with('#') => {
                    entity[1..].parse::<u32>().ok().and_then(char::from_u32)
                }
                _ => None,
            };
            if let Some(c) = decoded {
                out.push(c);
            } else {
                out.push('&');
                out.push_str(&entity);
                out.push(';');
            }
        } else {
            out.push('&');
            out.push_str(&entity);
        }
    }
    out
}

/// Split at sentence punctuation followed by whitespace and a capital/quote,
/// except where the period belongs to a known abbreviation or decimal number.
/// Sentences outside the 25–400 character range are discarded per the ingest plan.
pub fn split_sentences(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut starts = vec![0usize];
    for i in 0..chars.len() {
        if !matches!(chars[i], '.' | '!' | '?') || i + 1 >= chars.len() {
            continue;
        }
        let next = i + 1;
        if !chars[next].is_whitespace() {
            continue;
        }
        if chars[i] == '.' && (is_decimal(&chars, i) || abbreviation_before(&chars, i)) {
            continue;
        }
        let mut j = next;
        while j < chars.len() && chars[j].is_whitespace() {
            j += 1;
        }
        if j == chars.len()
            || chars[j].is_uppercase()
            || matches!(chars[j], '"' | '\'' | '“' | '‘' | '(')
        {
            starts.push(j);
        }
    }
    let mut output = Vec::new();
    for (n, start) in starts.iter().enumerate() {
        let end = if n + 1 < starts.len() {
            starts[n + 1]
        } else {
            chars.len()
        };
        let sentence: String = chars[*start..end].iter().collect();
        let sentence = sentence.trim();
        let count = sentence.chars().count();
        if (tuning::SENTENCE_MIN_CHARS..=tuning::SENTENCE_MAX_CHARS).contains(&count) {
            output.push(sentence.to_owned());
        }
    }
    output
}

fn is_decimal(chars: &[char], i: usize) -> bool {
    i > 0 && i + 1 < chars.len() && chars[i - 1].is_ascii_digit() && chars[i + 1].is_ascii_digit()
}

fn abbreviation_before(chars: &[char], i: usize) -> bool {
    // a.m. and p.m. are commonly written as a single token, so the preceding
    // whitespace lands between their two letter groups during token scanning.
    if i >= 3 {
        let tail: String = chars[i - 3..=i].iter().collect();
        let boundary = i == 3 || chars[i - 4].is_whitespace();
        if boundary && matches!(tail.to_ascii_lowercase().as_str(), "a.m." | "p.m.") {
            return true;
        }
    }
    // Do not split initialisms such as U.S. or e.g. at their internal periods.
    if i > 0 && chars[i - 1].is_alphabetic() {
        let mut j = i + 1;
        while j < chars.len() && chars[j].is_alphabetic() {
            j += 1;
        }
        if j < chars.len() && chars[j] == '.' && j > i + 1 {
            return true;
        }
    }
    let start = (0..i)
        .rev()
        .find(|&n| chars[n].is_whitespace())
        .map_or(0, |n| n + 1);
    let token: String = chars[start..=i].iter().collect();
    let lower = token.to_ascii_lowercase();
    [
        "mr.", "mrs.", "ms.", "dr.", "prof.", "st.", "u.s.", "e.g.", "i.e.", "a.m.", "p.m.", "vs.",
        "etc.",
    ]
    .iter()
    .any(|abbr| lower == *abbr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sentence(a: &str, b: &str) -> String {
        format!("{a}. {b}.")
    }

    #[test]
    fn sentence_splitter_cases() {
        let cases = [
            sentence(
                "The council approved the plan",
                "Construction starts next year",
            ),
            sentence(
                "The report was released today",
                "Officials welcomed the findings",
            ),
            sentence(
                "Police closed the bridge after a serious collision",
                "Traffic resumed at noon after crews cleared the roadway",
            ),
            sentence(
                "The team won the final after extra time",
                "Fans celebrated downtown with players after the match",
            ),
            sentence(
                "The company opened a new office in Victoria",
                "It will hire 40 workers over the next year",
            ),
            sentence(
                "The bill passed its second reading",
                "A final vote is scheduled Friday",
            ),
            sentence(
                "The storm reached the coast",
                "Residents were told to stay indoors",
            ),
            sentence(
                "The museum announced an exhibit about local history",
                "It opens in June and runs through the summer",
            ),
            sentence(
                "The agency published its review",
                "The minister will respond tomorrow",
            ),
            sentence(
                "The project received funding from the provincial government",
                "Work begins this summer after the final permits arrive",
            ),
            sentence(
                "The airport added two routes",
                "The flights begin in October",
            ),
            sentence(
                "The school board changed the calendar",
                "Classes will start earlier",
            ),
            sentence(
                "The company reported higher sales",
                "Its shares rose in afternoon trading",
            ),
            sentence(
                "The province introduced new rules",
                "They take effect next month",
            ),
            sentence(
                "The harbour reopened after repairs",
                "Boats returned before sunset",
            ),
            sentence(
                "The court issued its decision",
                "Lawyers will review the ruling",
            ),
            sentence(
                "The survey included 1.5 million responses",
                "Researchers released the results",
            ),
            sentence(
                "Dr. Singh led the university's new research study",
                "The team published its findings in a science journal",
            ),
            sentence(
                "Mr. Lee spoke at the event",
                "He answered questions afterward",
            ),
            sentence(
                "The U.S. agency issued guidance",
                "Officials said it starts today",
            ),
            sentence(
                "The briefing begins each day at 9 a.m. Pacific time for local staff",
                "The minister spoke for an hour after the meeting",
            ),
            sentence(
                "The public meeting starts at 7 p.m. Pacific time in Vancouver",
                "The chair will take questions from residents after the presentation",
            ),
            sentence(
                "The update includes e.g. new safety checks",
                "The checks begin Monday at all regional offices",
            ),
            sentence(
                "The memo explains i.e. the revised process",
                "Staff will receive training",
            ),
            sentence(
                "The route passes St. John's station",
                "Trains stop there every hour",
            ),
        ];
        for input in cases {
            let split = split_sentences(&input);
            assert_eq!(split.len(), 2, "{input:?} => {split:?}");
        }
    }

    #[test]
    fn cleanup_removes_boilerplate_and_decodes_feed_html() {
        assert_eq!(
            normalize_text("Advertisement\nA useful paragraph long enough to keep."),
            "A useful paragraph long enough to keep."
        );
        assert_eq!(
            strip_html("<p>News &amp; weather</p><script>ignore me</script>"),
            " News & weather "
        );
    }

    #[test]
    fn extracts_synthetic_article_fixture_with_readability() {
        let text = extract_page_html(
            include_str!("../tests/fixtures/article.html"),
            "https://fixture.example.test/town-harbour-plan",
        )
        .unwrap();
        assert!(text.chars().count() >= tuning::ARTICLE_FALLBACK_MIN_CHARS);
        assert!(text.contains("eastern harbour wall"));
        assert!(!text.contains("Share this story"));
    }

    #[test]
    fn blocked_article_fixture_falls_back_to_feed_content() {
        let page = extract_page_html(
            include_str!("../tests/fixtures/blocked.html"),
            "https://fixture.example.test/blocked",
        );
        if let Ok(text) = &page {
            assert!(text.chars().count() < tuning::ARTICLE_FALLBACK_MIN_CHARS);
        }
        let fallback = "<p>The publisher's access page did not contain the article. The feed summary reports that crews will inspect the north bridge next week, publish a schedule, and keep residents informed while repairs continue.</p>";
        let (text, source) = choose_article_text(page, Some(fallback));
        assert_eq!(source, TextSource::Feed);
        assert!(text.contains("crews will inspect the north bridge"));
        assert!(!text.is_empty());
    }
}
