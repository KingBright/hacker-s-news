pub mod feed_fetcher;
pub mod model;
pub mod normalizer;
pub mod opml;
pub mod source;

pub use feed_fetcher::{fetch_feed_entries, fetch_url_bytes, FeedFetchOptions};
pub use model::FetchedEntry;
pub use normalizer::clean_text_for_processing;
pub use opml::parse_opml_sources;
pub use source::{ContentSource, ProductLine};
