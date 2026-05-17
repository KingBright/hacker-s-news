use super::source::{ContentSource, ProductLine};
use anyhow::Result;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

pub fn parse_opml_sources(
    content: &[u8],
    source_group: Option<&str>,
) -> Result<Vec<ContentSource>> {
    let mut reader = Reader::from_reader(content);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut sources = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Empty(element) | Event::Start(element)
                if element.name().as_ref() == b"outline" =>
            {
                if let Some(source) = outline_to_source(&reader, &element, source_group)? {
                    sources.push(source);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(sources)
}

fn outline_to_source(
    reader: &Reader<&[u8]>,
    element: &BytesStart<'_>,
    source_group: Option<&str>,
) -> Result<Option<ContentSource>> {
    let mut title = None;
    let mut xml_url = None;
    let mut html_url = None;

    for attr in element.attributes().with_checks(false) {
        let attr = attr?;
        let key = attr.key.as_ref();
        let value = attr
            .decode_and_unescape_value(reader.decoder())?
            .to_string();
        match key {
            b"title" | b"text" if title.is_none() => title = Some(value),
            b"xmlUrl" | b"xmlurl" => xml_url = Some(value),
            b"htmlUrl" | b"htmlurl" => html_url = Some(value),
            _ => {}
        }
    }

    let Some(url) = xml_url else {
        return Ok(None);
    };

    let id = stable_source_id(&url);
    let mut source = ContentSource::new(
        id,
        title.unwrap_or_else(|| html_url.unwrap_or_else(|| url.clone())),
        url,
        ProductLine::CuratedFeed,
    );
    source.source_group = source_group.map(str::to_string);
    Ok(Some(source))
}

fn stable_source_id(url: &str) -> String {
    let slug = url
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_ascii_lowercase();

    if slug.is_empty() {
        "opml-source".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_outline_xml_urls() {
        let opml = br#"<?xml version="1.0"?>
<opml version="2.0">
  <body>
    <outline text="Group">
      <outline text="Karpathy" title="Karpathy" xmlUrl="https://karpathy.bearblog.dev/feed/"/>
      <outline text="No feed" htmlUrl="https://example.com"/>
    </outline>
  </body>
</opml>"#;

        let sources = parse_opml_sources(opml, Some("test")).expect("parse opml");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].name, "Karpathy");
        assert_eq!(sources[0].url, "https://karpathy.bearblog.dev/feed/");
        assert_eq!(sources[0].source_group.as_deref(), Some("test"));
    }
}
