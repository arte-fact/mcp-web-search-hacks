use scraper::{Html, Selector};

#[derive(Debug, serde::Serialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

pub fn html_to_text(html: &str, width: usize) -> String {
    html2text::from_read(html.as_bytes(), width).unwrap_or_else(|_| html.to_string())
}

pub fn parse_duckduckgo_results(html: &str) -> Vec<SearchResult> {
    let document = Html::parse_document(html);
    let result_selector = Selector::parse(".result").expect("valid selector");
    let title_selector = Selector::parse(".result__a").expect("valid selector");
    let snippet_selector = Selector::parse(".result__snippet").expect("valid selector");

    document
        .select(&result_selector)
        .filter_map(|result| {
            let title_el = result.select(&title_selector).next()?;
            let title = title_el.text().collect::<String>().trim().to_string();
            if title.is_empty() {
                return None;
            }
            let url = title_el
                .value()
                .attr("href")
                .unwrap_or_default()
                .to_string();
            let snippet = result
                .select(&snippet_selector)
                .next()
                .map(|el| el.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            Some(SearchResult {
                title,
                url,
                snippet,
            })
        })
        .collect()
}
