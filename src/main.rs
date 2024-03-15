use std::io::IsTerminal;

use error_stack::*;
use futures::StreamExt as _;
use scraper::*;

#[derive(thiserror::Error, Debug)]
#[error("Could not extract RSS feed")]
pub struct RssExtractError;

type Result<T, E = Report<RssExtractError>> = std::result::Result<T, E>;

use clap::*;
#[derive(Debug, Clone, Parser)]
pub struct Cli {
    #[clap(required = true, help = "The urls of the youtube channels")]
    pub urls: Vec<String>,
}

#[tokio::main]
pub async fn main() -> Result<()> {
    let matches = Cli::parse();

    let mut errors = Option::<Report<RssExtractError>>::None;
    let results: Vec<[String; 2]> = futures::stream::iter(matches.urls)
        .map(|url| async move { (url.to_string(), rss_extract_from_url(&url).await) })
        .buffer_unordered(10)
        .collect::<Vec<(String, Result<String>)>>()
        .await
        .into_iter()
        .scan(&mut errors, |acc, (name, rss)| match rss {
            Ok(rss) => Some(Some([name, rss])),
            Err(err) => match acc {
                Some(ref mut errors) => {
                    errors.extend_one(err);
                    Some(None)
                }
                None => {
                    **acc = Some(err);
                    Some(None)
                }
            },
        })
        .flatten()
        .collect();

    let mut table = comfy_table::Table::new();
    if std::io::stdout().is_terminal() {
        Report::set_charset(fmt::Charset::Utf8);
        Report::set_color_mode(fmt::ColorMode::Color);
        table
            .load_preset(comfy_table::presets::UTF8_FULL.replace('┆', "│").as_str())
            .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
            .set_content_arrangement(comfy_table::ContentArrangement::Dynamic);
    } else {
        table
            .load_preset(comfy_table::presets::NOTHING)
            .set_content_arrangement(comfy_table::ContentArrangement::Dynamic);
    }

    table.set_header(vec!["Youtube Channel", "RSS Feed"]);

    use std::io::Write;
    table.add_rows(results);
    writeln!(std::io::stdout(), "{}", table).change_context(RssExtractError)?;
    if let Some(errors) = errors {
        writeln!(std::io::stderr(), "{:?}", errors).change_context(RssExtractError)?;
    }
    Ok(())
}

async fn rss_extract_from_url(arg: &str) -> Result<String> {
    let html = reqwest::get(arg)
        .await
        .change_context(RssExtractError)
        .attach_printable_lazy(|| format!("Unable to query the youtube channel url: {arg}"))?
        .text()
        .await
        .change_context(RssExtractError)
        .attach_printable("Failed to parse youtube response as string")?;
    let results = rss_extractor(html);
    if results.is_empty() {
        return Err(Report::new(RssExtractError))
            .attach_printable_lazy(|| format!("Unable to find any rss feed for channel {arg}"))
            .attach_printable("Are you sure this is a youtube channel ?")?;
    } else if results.len() > 1 {
        eprintln!("Found more than one RSS feed for the channel, using the first one");
    }
    Ok(results[0].to_string()) // is safe because we checked for empty
}

fn rss_extractor(html: impl AsRef<str>) -> Vec<String> {
    let html = Html::parse_document(html.as_ref());
    let selector = Selector::parse("script").expect("Could not parse selector");
    let rss_finder = memchr::memmem::Finder::new("rssUrl");
    html.select(&selector)
        .filter_map(|element| {
            let value = element.inner_html();
            let offset = rss_finder.find(value.as_bytes())?;
            // "[r]ssUrl":"http...
            let offset = offset + memchr::memchr(b':', value[offset..].as_bytes())?;
            // "rssUrl" [:] "http...
            let offset = offset + memchr::memchr(b'"', value[offset..].as_bytes())?;
            // "rssUrl" : ["]http...
            let offset_end = memchr::memchr(b'"', value[offset + 1..].as_bytes())?;
            // "rssUrl" : "http... ["]
            let value = &value[offset + 1..offset + offset_end + 1];
            Some(value.to_string())
        })
        .collect()
}
