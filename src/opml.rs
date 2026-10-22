//! Small, dependency-light OPML import and export helpers.

use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use std::io::Cursor;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpmlFeed {
    pub url: String,
    pub title: Option<String>,
    /// Folder path, with nested OPML folders joined by `/`.
    pub folder: Option<String>,
}

#[derive(Debug)]
pub struct OpmlError(pub String);

impl std::fmt::Display for OpmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for OpmlError {}

/// Parse feed outlines from an OPML document. Non-feed outlines are treated as folders.
pub fn parse_opml(input: &str) -> Result<Vec<OpmlFeed>, OpmlError> {
    let mut reader = Reader::from_str(input);
    reader.config_mut().trim_text(true);
    let mut folders: Vec<String> = Vec::new();
    // Track paired feed and folder outlines alike so a closing feed tag cannot
    // accidentally pop its parent folder.
    let mut outline_stack: Vec<bool> = Vec::new();
    let mut feeds = Vec::new();
    let mut depth = 0usize;
    let mut root_seen = false;
    let mut root_closed = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                if depth == 0 {
                    if root_seen || !event.local_name().as_ref().eq_ignore_ascii_case(b"opml") {
                        return Err(OpmlError(
                            "document root must be a single <opml> element".into(),
                        ));
                    }
                    root_seen = true;
                }
                if root_closed {
                    return Err(OpmlError("content after closing <opml> root".into()));
                }
                depth += 1;
                if event.local_name().as_ref().eq_ignore_ascii_case(b"outline") {
                    let attrs = attributes(&reader, &event)?;
                    if let Some(url) = attrs.get("xmlurl").filter(|s| !s.trim().is_empty()) {
                        feeds.push(feed_from_attrs(url, &attrs, &folders));
                        outline_stack.push(false);
                    } else if let Some(name) = attrs.get("text").or_else(|| attrs.get("title")) {
                        if !name.trim().is_empty() {
                            folders.push(name.trim().to_owned());
                            outline_stack.push(true);
                        } else {
                            outline_stack.push(false);
                        }
                    } else {
                        outline_stack.push(false);
                    }
                }
            }
            Ok(Event::Empty(event)) => {
                if depth == 0 {
                    return Err(OpmlError("content outside <opml> root".into()));
                }
                if event.local_name().as_ref().eq_ignore_ascii_case(b"outline") {
                    let attrs = attributes(&reader, &event)?;
                    if let Some(url) = attrs.get("xmlurl").filter(|s| !s.trim().is_empty()) {
                        feeds.push(feed_from_attrs(url, &attrs, &folders));
                    }
                }
            }
            Ok(Event::End(event)) => {
                if depth == 0 {
                    return Err(OpmlError("unexpected closing XML element".into()));
                }
                if event.local_name().as_ref().eq_ignore_ascii_case(b"outline")
                    && outline_stack.pop() == Some(true)
                {
                    folders.pop();
                }
                depth -= 1;
                if depth == 0 {
                    root_closed = true;
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => (),
            Err(e) => return Err(OpmlError(format!("invalid OPML XML: {e}"))),
        }
    }
    if !root_seen || !root_closed || depth != 0 {
        return Err(OpmlError("incomplete OPML document".into()));
    }
    Ok(feeds)
}

fn attributes<R: std::io::BufRead>(
    reader: &Reader<R>,
    event: &BytesStart<'_>,
) -> Result<std::collections::HashMap<String, String>, OpmlError> {
    let mut out = std::collections::HashMap::new();
    for attr in event.attributes() {
        let attr = attr.map_err(|e| OpmlError(format!("invalid OPML attribute: {e}")))?;
        let key = String::from_utf8_lossy(attr.key.local_name().as_ref()).to_ascii_lowercase();
        let value = attr
            .decode_and_unescape_value(reader.decoder())
            .map_err(|e| OpmlError(format!("invalid OPML attribute value: {e}")))?
            .into_owned();
        out.insert(key, value);
    }
    Ok(out)
}

fn feed_from_attrs(
    url: &str,
    attrs: &std::collections::HashMap<String, String>,
    folders: &[String],
) -> OpmlFeed {
    let title = attrs
        .get("title")
        .or_else(|| attrs.get("text"))
        .filter(|s| !s.trim().is_empty())
        .cloned();
    let folder = if folders.is_empty() {
        None
    } else {
        Some(folders.join("/"))
    };
    OpmlFeed {
        url: url.trim().to_owned(),
        title,
        folder,
    }
}

/// Export feeds as a standards-shaped OPML 2.0 document. Folders are preserved
/// as nested outline groups; feeds without a title use their URL as display text.
pub fn export_opml(feeds: &[OpmlFeed]) -> Result<String, OpmlError> {
    let mut writer = Writer::new_with_indent(Cursor::new(Vec::new()), b' ', 2);
    writer
        .write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))
        .map_err(xml_error)?;
    writer
        .write_event(Event::Start(
            BytesStart::new("opml").with_attributes([("version", "2.0")]),
        ))
        .map_err(xml_error)?;
    writer
        .write_event(Event::Start(BytesStart::new("head")))
        .map_err(xml_error)?;
    writer
        .write_event(Event::Start(BytesStart::new("title")))
        .map_err(xml_error)?;
    writer
        .write_event(Event::Text(BytesText::new("Wagwan subscriptions")))
        .map_err(xml_error)?;
    writer
        .write_event(Event::End(BytesEnd::new("title")))
        .map_err(xml_error)?;
    writer
        .write_event(Event::End(BytesEnd::new("head")))
        .map_err(xml_error)?;
    writer
        .write_event(Event::Start(BytesStart::new("body")))
        .map_err(xml_error)?;

    // Generate each folder once and keep the input order within each group.
    let mut tree = Folder::default();
    for feed in feeds {
        let mut node = &mut tree;
        if let Some(folder) = &feed.folder {
            for part in folder.split('/').filter(|s| !s.trim().is_empty()) {
                node = node.children.entry(part.trim().to_owned()).or_default();
            }
        }
        node.feeds.push(feed);
    }
    write_folder(&mut writer, &tree)?;
    writer
        .write_event(Event::End(BytesEnd::new("body")))
        .map_err(xml_error)?;
    writer
        .write_event(Event::End(BytesEnd::new("opml")))
        .map_err(xml_error)?;
    String::from_utf8(writer.into_inner().into_inner()).map_err(|e| OpmlError(e.to_string()))
}

#[derive(Default)]
struct Folder<'a> {
    children: std::collections::BTreeMap<String, Folder<'a>>,
    feeds: Vec<&'a OpmlFeed>,
}

fn write_folder(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    folder: &Folder<'_>,
) -> Result<(), OpmlError> {
    for (name, child) in &folder.children {
        let mut group = BytesStart::new("outline");
        group.push_attribute(("text", name.as_str()));
        group.push_attribute(("title", name.as_str()));
        writer.write_event(Event::Start(group)).map_err(xml_error)?;
        write_folder(writer, child)?;
        writer
            .write_event(Event::End(BytesEnd::new("outline")))
            .map_err(xml_error)?;
    }
    for feed in &folder.feeds {
        let title = feed.title.as_deref().unwrap_or(&feed.url);
        let mut outline = BytesStart::new("outline");
        outline.push_attribute(("text", title));
        outline.push_attribute(("title", title));
        outline.push_attribute(("type", "rss"));
        outline.push_attribute(("xmlUrl", feed.url.as_str()));
        writer
            .write_event(Event::Empty(outline))
            .map_err(xml_error)?;
    }
    Ok(())
}

fn xml_error(error: std::io::Error) -> OpmlError {
    OpmlError(format!("could not write OPML: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_groups_and_feed_titles() {
        let xml = r#"<opml version="2.0"><body><outline text="News"><outline text="Local"><outline text="Paper" title="The Paper" type="rss" xmlUrl="https://example.com/feed?a=1&amp;b=2"/></outline></outline><outline text="Plain" xmlUrl="https://example.net/rss"/></body></opml>"#;
        let feeds = parse_opml(xml).unwrap();
        assert_eq!(
            feeds,
            vec![
                OpmlFeed {
                    url: "https://example.com/feed?a=1&b=2".into(),
                    title: Some("The Paper".into()),
                    folder: Some("News/Local".into())
                },
                OpmlFeed {
                    url: "https://example.net/rss".into(),
                    title: Some("Plain".into()),
                    folder: None
                },
            ]
        );
    }

    #[test]
    fn parses_nested_encoded_fixture() {
        let feeds = parse_opml(include_str!("../tests/fixtures/nested-opml.xml")).unwrap();
        assert_eq!(feeds.len(), 3);
        assert_eq!(feeds[0].title.as_deref(), Some("Example Journal"));
        assert_eq!(
            feeds[0].url,
            "https://example.org/world.xml?edition=full&lang=en"
        );
        assert_eq!(feeds[0].folder.as_deref(), Some("World & Politics"));
        assert_eq!(
            feeds[1].folder.as_deref(),
            Some("World & Politics/Regional")
        );
        assert_eq!(feeds[2].title.as_deref(), Some("Signal & Noise"));
        assert_eq!(feeds[2].folder.as_deref(), Some("Technology"));
    }

    #[test]
    fn export_round_trips_feed_data_and_escapes_attributes() {
        let feeds = vec![OpmlFeed {
            url: "https://example.com/?a=1&b=2".into(),
            title: Some("A & B".into()),
            folder: Some("News/Local".into()),
        }];
        let xml = export_opml(&feeds).unwrap();
        assert!(xml.contains("&amp;"));
        assert_eq!(parse_opml(&xml).unwrap(), feeds);
    }

    #[test]
    fn malformed_xml_is_reported() {
        assert!(parse_opml("<opml><body>").is_err());
        assert!(parse_opml("<rss><body/></rss>").is_err());
        assert!(parse_opml("<opml/><opml/>").is_err());
    }

    #[test]
    fn paired_feed_outline_keeps_its_parent_folder() {
        let xml = r#"<opml><body><outline text="News"><outline text="Paper" xmlUrl="https://example.com/feed"></outline></outline></body></opml>"#;
        let feeds = parse_opml(xml).unwrap();
        assert_eq!(feeds[0].folder.as_deref(), Some("News"));
    }
}
