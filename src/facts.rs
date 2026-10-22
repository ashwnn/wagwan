//! Lightweight normalized fact tokens used to constrain story joins and measure novelty.

use std::collections::HashSet;

#[derive(Debug, Clone)]
struct Token {
    raw: String,
    start: usize,
    end: usize,
    word_index: usize,
    sentence_start: bool,
}

/// Extract normalized numbers, dates/times, multi-word names and meaningful quotations.
pub fn fact_tokens(text: &str) -> HashSet<String> {
    let tokens = scan_tokens(text);
    let mut facts = HashSet::new();
    add_numeric_facts(&tokens, &mut facts);
    add_dates_times(&tokens, text, &mut facts);
    add_capitalized_spans(&tokens, text, &mut facts);
    add_quotes(text, &mut facts);
    facts
}

/// Share of facts that were not already present in earlier originals.
pub fn uniqueness(current: &HashSet<String>, earlier_original_facts: &[HashSet<String>]) -> f32 {
    if current.is_empty() {
        return 0.0;
    }
    let old: HashSet<&str> = earlier_original_facts
        .iter()
        .flat_map(|set| set.iter().map(String::as_str))
        .collect();
    let new_count = current
        .iter()
        .filter(|fact| !old.contains(fact.as_str()))
        .count();
    new_count as f32 / current.len() as f32
}

fn scan_tokens(text: &str) -> Vec<Token> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut result = Vec::new();
    let mut start = None;
    let mut word_index = 0;
    let mut sentence_start = true;
    for i in 0..=chars.len() {
        let (index, ch) = chars.get(i).copied().unwrap_or((text.len(), ' '));
        let previous = i.checked_sub(1).and_then(|j| chars.get(j)).map(|(_, c)| *c);
        let next = chars.get(i + 1).map(|(_, c)| *c);
        let decimal_point = ch == '.'
            && previous.is_some_and(|c| c.is_ascii_digit())
            && next.is_some_and(|c| c.is_ascii_digit());
        let is_token = ch.is_alphanumeric()
            || matches!(ch, ',' | ':' | '%' | '\'' | '-')
            || matches!(ch, '$' | '£' | '€' | '¥')
            || decimal_point;
        if is_token {
            start.get_or_insert(index);
        } else if let Some(begin) = start.take() {
            let raw = &text[begin..index];
            if raw.chars().any(char::is_alphanumeric) {
                result.push(Token {
                    raw: raw.to_owned(),
                    start: begin,
                    end: index,
                    word_index,
                    sentence_start,
                });
                word_index += 1;
                sentence_start = false;
            }
        }
        if matches!(ch, '.' | '!' | '?') && !decimal_point {
            sentence_start = true;
        }
    }
    result
}

fn add_numeric_facts(tokens: &[Token], facts: &mut HashSet<String>) {
    let mut i = 0;
    while i < tokens.len() {
        let raw = tokens[i].raw.as_str();
        let (currency, number_text) = currency_prefix(raw);
        let Some(mut value) = parse_number(number_text) else {
            i += 1;
            continue;
        };
        let mut consumed = 1;
        let mut unit = "";
        if let Some(next) = tokens.get(i + 1) {
            let suffix = next.raw.trim_end_matches('.').to_ascii_lowercase();
            match suffix.as_str() {
                "thousand" | "k" => {
                    value *= 1_000.0;
                    consumed = 2;
                }
                "million" | "mn" | "m" => {
                    value *= 1_000_000.0;
                    consumed = 2;
                }
                "billion" | "bn" | "b" => {
                    value *= 1_000_000_000.0;
                    consumed = 2;
                }
                _ => {}
            }
        }
        if let Some(next) = tokens.get(i + consumed) {
            match next.raw.as_str() {
                "%" => {
                    unit = "%";
                    consumed += 1;
                }
                _ if next.raw.starts_with('%') => {
                    unit = "%";
                    consumed += 1;
                }
                _ if next.raw.eq_ignore_ascii_case("percentage")
                    && tokens
                        .get(i + consumed + 1)
                        .is_some_and(|token| token.raw.eq_ignore_ascii_case("points")) =>
                {
                    unit = "%";
                    consumed += 2;
                }
                _ if matches!(
                    next.raw.to_ascii_lowercase().as_str(),
                    "percent" | "percentage"
                ) =>
                {
                    unit = "%";
                    consumed += 1;
                }
                _ => {}
            }
        }
        if unit.is_empty() && tokens[i].raw.ends_with('%') {
            unit = "%";
        }
        let normalized = format_number(value);
        let currency = currency
            .or_else(|| tokens.get(i + consumed).and_then(|t| currency_code(&t.raw)))
            .or_else(|| {
                i.checked_sub(1)
                    .and_then(|previous| currency_code(&tokens[previous].raw))
            });
        let fact = if let Some(code) = currency {
            format!("number:{code}:{normalized}{unit}")
        } else {
            format!("number:{normalized}{unit}")
        };
        facts.insert(fact);
        i += consumed;
    }
}

fn currency_prefix(s: &str) -> (Option<&'static str>, &str) {
    let upper = s.to_ascii_uppercase();
    for (prefix, code) in [
        ("US$", "USD"),
        ("$US", "USD"),
        ("CA$", "CAD"),
        ("C$", "CAD"),
        ("$CA", "CAD"),
        ("$C", "CAD"),
        ("AU$", "AUD"),
        ("A$", "AUD"),
        ("$AU", "AUD"),
        ("$A", "AUD"),
        ("$CAD", "CAD"),
        ("CAD$", "CAD"),
        ("$AUD", "AUD"),
        ("AUD$", "AUD"),
        ("$USD", "USD"),
        ("USD$", "USD"),
    ] {
        if upper.starts_with(prefix) {
            return (Some(code), &s[prefix.len()..]);
        }
    }
    if let Some(rest) = s.strip_prefix('$') {
        (Some("USD"), rest)
    } else if let Some(rest) = s.strip_prefix('£') {
        (Some("GBP"), rest)
    } else if let Some(rest) = s.strip_prefix('€') {
        (Some("EUR"), rest)
    } else if let Some(rest) = s.strip_prefix('¥') {
        (Some("JPY"), rest)
    } else {
        (None, s)
    }
}

fn currency_code(s: &str) -> Option<&'static str> {
    match s
        .trim_matches(|c: char| !c.is_alphabetic() && c != '$')
        .to_ascii_uppercase()
        .as_str()
    {
        "USD" | "US" | "US$" | "$US" => Some("USD"),
        "CAD" | "CA$" | "C$" | "$CA" | "$C" | "$CAD" => Some("CAD"),
        "AUD" | "AU$" | "A$" | "$AU" | "$A" | "$AUD" => Some("AUD"),
        "GBP" => Some("GBP"),
        "EUR" => Some("EUR"),
        "JPY" => Some("JPY"),
        _ => None,
    }
}

fn parse_number(s: &str) -> Option<f64> {
    let cleaned: String = s.chars().filter(|ch| *ch != ',').collect();
    let cleaned = cleaned.trim_end_matches('%');
    if cleaned.is_empty() || !cleaned.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    cleaned
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn format_number(value: f64) -> String {
    if value.fract().abs() < 0.000_001 {
        format!("{:.0}", value)
    } else {
        format!("{value:.6}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

fn add_dates_times(tokens: &[Token], source: &str, facts: &mut HashSet<String>) {
    const MONTHS: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    const SHORT_MONTHS: [&str; 13] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "sept", "oct", "nov", "dec",
    ];
    for (i, token) in tokens.iter().enumerate() {
        let lower = token
            .raw
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_ascii_lowercase();
        let month = MONTHS.iter().position(|month| *month == lower).or_else(|| {
            SHORT_MONTHS
                .iter()
                .position(|month| *month == lower)
                .map(|month| if month > 8 { month - 1 } else { month })
        });
        if let Some(month) = month {
            let first = tokens.get(i + 1).and_then(|t| digits(&t.raw));
            let second = tokens.get(i + 2).and_then(|t| digits(&t.raw));
            if let Some(year) = first.as_ref().filter(|value| value.len() == 4) {
                facts.insert(format!("date:{year}-{:02}", month + 1));
            } else if let Some(day) = first
                .as_ref()
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|day| (1..=31).contains(day))
            {
                let year = second.filter(|value| value.len() == 4);
                if let Some(year) = year {
                    facts.insert(format!("date:{year}-{:02}-{day:02}", month + 1));
                } else {
                    facts.insert(format!("date:--{:02}-{day:02}", month + 1));
                }
            }
        }
        if let Some((year, month, day)) = parse_iso_date(&token.raw) {
            facts.insert(format!("date:{year:04}-{month:02}-{day:02}"));
        }
        if let (Some(month), Some(day), Some(year)) = (
            digits(&token.raw).and_then(|value| value.parse::<u32>().ok()),
            tokens
                .get(i + 1)
                .and_then(|next| digits(&next.raw))
                .and_then(|value| value.parse::<u32>().ok()),
            tokens
                .get(i + 2)
                .and_then(|next| digits(&next.raw))
                .filter(|value| value.len() == 4),
        ) {
            let separator_one = &source[token.end..tokens[i + 1].start];
            let separator_two = &source[tokens[i + 1].end..tokens[i + 2].start];
            if (separator_one.contains('/') || separator_one.contains('-'))
                && (separator_two.contains('/') || separator_two.contains('-'))
                && (1..=12).contains(&month)
                && (1..=31).contains(&day)
            {
                facts.insert(format!("date:{year}-{:02}-{day:02}", month));
            }
        }
        let meridiem = tokens.get(i + 1).and_then(|next| {
            let marker = next.raw.to_ascii_lowercase();
            if marker == "am" || marker == "pm" {
                Some(marker)
            } else if (marker == "a" || marker == "p")
                && tokens
                    .get(i + 2)
                    .is_some_and(|third| third.raw.eq_ignore_ascii_case("m"))
            {
                Some(format!("{marker}m"))
            } else {
                None
            }
        });
        if token.raw.contains(':') || meridiem.is_some() {
            let mut clock = token.raw.clone();
            if let Some(marker) = meridiem {
                clock.push_str(&marker);
            }
            if let Some(time) = parse_clock(&clock) {
                facts.insert(format!("time:{time}"));
            }
        }
    }
}

fn parse_iso_date(raw: &str) -> Option<(u32, u32, u32)> {
    let date = raw.split_once('T').map_or(raw, |(date, _)| date);
    let mut parts = date.split('-');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next()?.parse().ok()?;
    let day = parts
        .next()?
        .trim_end_matches(|c: char| !c.is_ascii_digit())
        .parse()
        .ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

fn parse_clock(raw: &str) -> Option<String> {
    let raw = raw.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != ':');
    let lower = raw.to_ascii_lowercase();
    let (clock, meridiem) = if let Some(clock) = lower.strip_suffix("am") {
        (clock, Some(false))
    } else if let Some(clock) = lower.strip_suffix("pm") {
        (clock, Some(true))
    } else {
        (lower.as_str(), None)
    };
    let (hour, minute) = clock.split_once(':').unwrap_or((clock, "00"));
    let mut hour = hour.parse::<u32>().ok()?;
    let minute = minute.get(..2)?.parse::<u32>().ok()?;
    if minute > 59 {
        return None;
    }
    if let Some(pm) = meridiem {
        if hour == 0 || hour > 12 {
            return None;
        }
        hour = (hour % 12) + u32::from(pm) * 12;
    } else if hour > 23 {
        return None;
    }
    Some(format!("{hour:02}:{minute:02}"))
}

fn digits(raw: &str) -> Option<String> {
    let value: String = raw.chars().take_while(|c| c.is_ascii_digit()).collect();
    (!value.is_empty()).then_some(value)
}

fn add_capitalized_spans(tokens: &[Token], source: &str, facts: &mut HashSet<String>) {
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i].sentence_start || !is_capitalized(&tokens[i].raw) {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < tokens.len()
            && is_capitalized(&tokens[i].raw)
            && adjacent_words(&tokens[i - 1], &tokens[i], source)
        {
            i += 1;
        }
        if i - start >= 2 {
            let phrase = tokens[start].start..tokens[i - 1].end;
            facts.insert(format!(
                "name:{}",
                source[phrase]
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_lowercase()
            ));
        }
    }
}

fn adjacent_words(a: &Token, b: &Token, source: &str) -> bool {
    source[a.end..b.start].chars().all(char::is_whitespace) && b.word_index == a.word_index + 1
}

fn is_capitalized(word: &str) -> bool {
    let chars: Vec<char> = word.chars().filter(|c| c.is_alphabetic()).collect();
    matches!(chars.first(), Some(c) if c.is_uppercase())
        && (chars.iter().any(|c| c.is_lowercase()) || chars.len() >= 2)
}

fn add_quotes(text: &str, facts: &mut HashSet<String>) {
    for delimiter in ['"', '“', '‘', '«'] {
        let close = match delimiter {
            '“' => '”',
            '‘' => '’',
            '«' => '»',
            other => other,
        };
        let mut remaining = text;
        while let Some(open) = remaining.find(delimiter) {
            let after_open = open + delimiter.len_utf8();
            let Some(end_rel) = remaining[after_open..].find(close) else {
                break;
            };
            let quoted = remaining[after_open..after_open + end_rel].trim();
            if quoted.split_whitespace().count() >= 3 {
                let normalized = quoted
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_lowercase();
                facts.insert(format!("quote:{normalized}"));
            }
            remaining = &remaining[after_open + end_rel + close.len_utf8()..];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_commas_magnitude_currency_and_percentages() {
        let facts = fact_tokens(
            "Revenue rose to $3.2 million; the prior figure was USD 3,200,000, up 12.5 percent.",
        );
        assert!(facts.contains("number:USD:3200000"));
        assert!(facts.contains("number:12.5%"));
    }

    #[test]
    fn extracts_date_time_name_and_meaningful_quote_tokens() {
        let facts = fact_tokens(
            "Officials said Acme Corporation met on March 4, 2025 at 3:05 p.m. and stated, \"we expect another strong quarter\".",
        );
        assert!(facts.contains("date:2025-03-04"), "{facts:?}");
        assert!(facts.contains("time:15:05"), "{facts:?}");
        assert!(facts.contains("name:acme corporation"), "{facts:?}");
        assert!(
            facts.contains("quote:we expect another strong quarter"),
            "{facts:?}"
        );
    }

    #[test]
    fn suppresses_sentence_initial_capitalization_and_short_quotes() {
        let facts =
            fact_tokens("London opened late. Paris stayed open. A brief quote: \"all good\".");
        assert!(!facts.contains("name:london"));
        assert!(!facts.contains("name:paris"));
        assert!(!facts.iter().any(|fact| fact.starts_with("quote:")));
    }

    #[test]
    fn uniqueness_compares_only_previously_seen_original_facts() {
        let current = HashSet::from(["number:5".to_owned(), "name:acme corp".to_owned()]);
        let earlier = HashSet::from(["number:5".to_owned()]);
        assert_eq!(uniqueness(&current, &[earlier]), 0.5);
        assert_eq!(uniqueness(&current, &[]), 1.0);
        assert_eq!(uniqueness(&HashSet::new(), &[]), 0.0);
    }
}
