use chrono::{DateTime, Utc};
use reqwest::header::USER_AGENT;
use scraper::{Html, Selector, Node, ElementRef, Element}; // Added Element
use serde::{Deserialize, Serialize};
use std::collections::HashSet; 
use std::fs;
use std::path::Path;
use url::Url; 
use regex::Regex; // Ensure this crate is in Cargo.toml

#[derive(Debug, Serialize, Deserialize)]
struct CrawlOutput {
    domain: String,
    root_url: String,
    crawl_timestamp: DateTime<Utc>,
    total_pages: usize,
    pages: Vec<PageData>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct PageData {
    url: String,
    title: String,
    content: PageContent,
    metadata: PageMetadata,
    links: Vec<LinkData>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct PageContent {
    full_text: String,
    headings: Vec<Heading>,
    paragraphs: Vec<String>,
    lists: Vec<String>,
    chunks: Vec<TextChunk>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Heading {
    level: u8,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_heading: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct TextChunk {
    chunk_id: String,
    text: String,
    char_start: usize,
    char_end: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    section_heading: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct PageMetadata {
    crawl_timestamp: DateTime<Utc>,
    depth: usize,
    word_count: usize,
    language: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct LinkData {
    text: String,
    href: String,
    link_type: LinkType,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
enum LinkType {
    Internal,
    External,
    Anchor,
}

struct Crawler {
    visited: HashSet<String>,
    pages: Vec<PageData>,
    domain: String,
    client: reqwest::blocking::Client,
}

// Helper struct to hold common selectors
struct Selectors {
    main_content: Vec<Selector>,
    boilerplate: Vec<Selector>,
    always_remove: Selector,
    cookie_banner_text: Vec<String>,
    json_like_pattern: Regex,
}

impl Selectors {
    fn new() -> Self {
        Selectors {
            main_content: vec![
                Selector::parse("main").unwrap(),
                Selector::parse("[role='main']").unwrap(),
                Selector::parse("#main-content").unwrap(),
                Selector::parse("#content").unwrap(),
                Selector::parse(".main-content").unwrap(),
                Selector::parse(".content").unwrap(),
                Selector::parse("article").unwrap(),
            ],
            boilerplate: vec![
                Selector::parse("header").unwrap(),
                Selector::parse("footer").unwrap(),
                Selector::parse("nav").unwrap(),
                Selector::parse("aside").unwrap(),
                Selector::parse(".cookie-banner").unwrap(), 
                Selector::parse("#cookie-consent").unwrap(), 
                Selector::parse(".sidebar").unwrap(),
                Selector::parse("div.secondary-navigation").unwrap(),
                Selector::parse("div.global-main-menu").unwrap(),
                Selector::parse("div.footer-menu").unwrap(),
                Selector::parse("div#onetrust-consent-sdk").unwrap(),
            ],
            always_remove: Selector::parse("script, style, noscript, svg, path, button, form, input, textarea, select, option, figure > figcaption, .visually-hidden, [aria-hidden='true']").unwrap(),
            cookie_banner_text: vec![
                "cookies we use cookies to help our site work".to_string(),
                "by accepting, you agree to cookies being stored".to_string(),
                "manage settings accept".to_string(),
            ],
            json_like_pattern: Regex::new(r#"\A\{.*\}\z|\A\[.*\]\z"#).unwrap(),
        }
    }
}


impl Crawler {
    fn new(root_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let url = Url::parse(root_url)?;
        let domain = url.host_str().unwrap_or("").to_string();
        
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        
        Ok(Crawler {
            visited: HashSet::new(),
            pages: Vec::new(),
            domain,
            client,
        })
    }
    
    fn crawl(&mut self, url: &str, depth: usize, max_depth: usize) {
        if depth >= max_depth || self.visited.contains(url) {
            return;
        }
        
        self.visited.insert(url.to_string());
        println!("Crawling: {} (depth: {})", url, depth);
        
        match self.scrape_page(url, depth) {
            Ok(page_data) => {
                if !page_data.content.full_text.trim().is_empty() || 
                   !page_data.content.paragraphs.is_empty() || 
                   !page_data.content.headings.is_empty() {
                    let links = page_data.links.clone();
                    self.pages.push(page_data);
                    
                    for link in links.iter() {
                        if matches!(link.link_type, LinkType::Internal) {
                            if let Some(filtered_url) = self.filter_url(url, &link.href) {
                                if !self.visited.contains(&filtered_url) {
                                    self.crawl(&filtered_url, depth + 1, max_depth);
                                }
                            }
                        }
                    }
                } else {
                    println!("Skipping page due to no meaningful content after cleaning: {}", url);
                }
            }
            Err(e) => {
                eprintln!("Error scraping {}: {}", url, e);
                self.pages.push(PageData {
                    url: url.to_string(),
                    title: "Failed to crawl".to_string(),
                    content: PageContent {
                        full_text: String::new(), headings: vec![], paragraphs: vec![], lists: vec![], chunks: vec![],
                    },
                    metadata: PageMetadata {
                        crawl_timestamp: Utc::now(), depth, word_count: 0, language: None, description: Some(format!("Error: {}", e)),
                    },
                    links: vec![],
                });
            }
        }
    }
    
    fn scrape_page(&self, url: &str, depth: usize) -> Result<PageData, Box<dyn std::error::Error>> {
        let response = self.client
            .get(url)
            .header(USER_AGENT, "Mozilla/5.0 (compatible; RustCrawler/1.0; +http://yourdomain.com/bot.html)")
            .send()?;
        
        let body = response.text()?;
        let document = Html::parse_document(&body);
        let selectors = Selectors::new();
        
        let title_selector = Selector::parse("title").unwrap();
        let title = document
            .select(&title_selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_else(|| "Untitled".to_string());
        
        let desc_selector = Selector::parse("meta[name=\"description\"]").unwrap();
        let description = document
            .select(&desc_selector)
            .next()
            .and_then(|el| el.value().attr("content"))
            .map(|s| s.trim().to_string());
        
        let main_content_element = self.find_main_content_element(&document, &selectors);

        let headings = self.extract_headings(&main_content_element, &selectors);
        let paragraphs = self.extract_paragraphs(&main_content_element, &selectors);
        let lists = self.extract_lists(&main_content_element, &selectors);
        let links = self.extract_links(&document, url)?;
        
        let full_text = self.build_full_text(&main_content_element, &selectors);
        let word_count = full_text.split_whitespace().count();
        
        let chunks = self.create_chunks(&full_text, &headings, url);
        
        Ok(PageData {
            url: url.to_string(),
            title,
            content: PageContent {
                full_text, headings, paragraphs, lists, chunks,
            },
            metadata: PageMetadata {
                crawl_timestamp: Utc::now(), depth, word_count, language: Some("en".to_string()), description,
            },
            links,
        })
    }

    fn find_main_content_element<'a>(&self, document: &'a Html, selectors: &Selectors) -> ElementRef<'a> {
        for selector in &selectors.main_content {
            if let Some(main_node) = document.select(selector).next() {
                return main_node;
            }
        }
        document.root_element()
    }

    fn is_skippable(&self, element: ElementRef, selectors: &Selectors) -> bool {
        if selectors.always_remove.matches(&element) {
            return true;
        }
        let mut current = Some(element);
        while let Some(el) = current {
            for bp_selector in &selectors.boilerplate {
                if bp_selector.matches(&el) {
                    return true;
                }
            }
            current = el.parent_element();
        }
        false
    }
    
    fn build_full_text<'a>(&self, main_content_element: &ElementRef<'a>, selectors: &Selectors) -> String {
        let mut text_parts: Vec<String> = Vec::new();
    
        fn extract_text_recursively(
            element: ElementRef,
            text_parts: &mut Vec<String>,
            selectors: &Selectors,
            depth: usize,
        ) {
            if depth > 50 || selectors.always_remove.matches(&element) {
                return;
            }
    
            if depth > 0 { 
                for bp_selector in &selectors.boilerplate {
                    if bp_selector.matches(&element) {
                        return;
                    }
                }
            }
    
            for node in element.children() {
                match node.value() {
                    Node::Text(text_node) => {
                        let original_text_trimmed = text_node.trim();
                        let processed_text_lower = original_text_trimmed.to_lowercase();
                        if !original_text_trimmed.is_empty() && 
                           !selectors.cookie_banner_text.iter().any(|p| processed_text_lower.contains(p)) &&
                           !selectors.json_like_pattern.is_match(original_text_trimmed) && 
                           !processed_text_lower.contains("permissionshash") {
                            text_parts.push(original_text_trimmed.to_string());
                        }
                    }
                    Node::Element(_) => {
                        if let Some(sub_element_ref) = ElementRef::wrap(node) {
                           extract_text_recursively(sub_element_ref, text_parts, selectors, depth + 1);
                        }
                    }
                    _ => {}
                }
            }
        }
    
        extract_text_recursively(*main_content_element, &mut text_parts, selectors, 0);
    
        text_parts.join(" ")
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }


    fn extract_headings<'a>(&self, main_content_element: &ElementRef<'a>, selectors: &Selectors) -> Vec<Heading> {
        let mut headings_data = Vec::new();
        let mut last_h1: Option<String> = None;
        let mut last_h2: Option<String> = None;
        let common_boilerplate_headings = ["navigation", "menu", "footer", "cookies", "search results", "search"];

        for level in 1..=6 {
            let selector_str = format!("h{}", level);
            
            // Perform parsing and handle error case by skipping iteration
            let heading_selector = match Selector::parse(&selector_str) {
                Ok(sel) => sel, // sel is Selector, which is 'static
                Err(e) => {
                    // selector_str is still alive here.
                    // e (SelectorErrorKind) borrows selector_str.
                    // We convert e to a string for potential logging, then e is dropped.
                    // The 'continue' ensures this whole path related to 'e' terminates.
                    let _error_string = e.to_string(); 
                    // eprintln!("Failed to parse heading selector for 'h{}': {}", level, _error_string);
                    continue; // Skip this iteration if parsing fails
                }
            };
            // If we reach here, selector_str is still alive, and heading_selector is a valid 'static Selector.
            // The temporary Result from Selector::parse and any SelectorErrorKind are gone.

            for element in main_content_element.select(&heading_selector) {
                if self.is_skippable(element, selectors) {
                    continue;
                }

                let text = element.text().collect::<String>().trim().to_string();
                let lower_text = text.to_lowercase();

                if text.is_empty() || common_boilerplate_headings.iter().any(|&s| lower_text.contains(s)) {
                    continue;
                }
                
                // Assuming Selector::parse("a") is infallible or handled appropriately
                let link_selector = Selector::parse("a").unwrap(); 
                let link_text_len: usize = element.select(&link_selector).flat_map(|a| a.text()).map(|t| t.len()).sum();
                if !text.is_empty() && link_text_len > text.len() / 2 && text.split_whitespace().count() < 5 {
                    continue;
                }

                let parent_heading = match level {
                    2 => last_h1.clone(),
                    3..=6 => last_h2.clone(),
                    _ => None,
                };
                
                headings_data.push(Heading {
                    level: level as u8,
                    text: text.clone(),
                    parent_heading,
                });
                
                match level {
                    1 => last_h1 = Some(text.clone()),
                    2 => last_h2 = Some(text.clone()), 
                    _ => {}
                }
            }
            // selector_str is dropped at the end of the loop iteration here.
        }
        headings_data
    }

    fn extract_paragraphs<'a>(&self, main_content_element: &ElementRef<'a>, selectors: &Selectors) -> Vec<String> {
        if let Ok(p_selector) = Selector::parse("p") {
            main_content_element
                .select(&p_selector)
                .filter_map(|el| {
                    if self.is_skippable(el, selectors) { return None; }

                    let text = el.text().collect::<String>().trim().to_string();
                    let lower_text = text.to_lowercase();

                    if text.is_empty() ||
                       selectors.cookie_banner_text.iter().any(|p| lower_text.contains(p)) ||
                       selectors.json_like_pattern.is_match(&text) ||
                       lower_text.contains("permissionshash") ||
                       lower_text.contains("skip to main content") ||
                       (el.select(&Selector::parse("a").unwrap()).next().is_some() && 
                        el.text().collect::<String>().trim().len() == el.select(&Selector::parse("a").unwrap()).next().unwrap().text().collect::<String>().trim().len() && 
                        text.split_whitespace().count() < 7)
                       {
                        return None;
                    }
                    Some(text)
                })
                .collect()
        } else {
            vec![]
        }
    }

    fn extract_lists<'a>(&self, main_content_element: &ElementRef<'a>, selectors: &Selectors) -> Vec<String> {
        let mut lists_text = Vec::new();
        if let Ok(li_selector) = Selector::parse("li") {
            for element in main_content_element.select(&li_selector) {
                if self.is_skippable(element, selectors) {
                    continue;
                }
                let text = element.text().collect::<String>().trim().to_string();
                let lower_text = text.to_lowercase();

                if text.is_empty() ||
                   selectors.cookie_banner_text.iter().any(|p| lower_text.contains(p)) ||
                   selectors.json_like_pattern.is_match(&text) ||
                   lower_text.contains("permissionshash") {
                    continue;
                }

                let total_text_len = text.len();
                let link_text_len: usize = element.select(&Selector::parse("a").unwrap()).flat_map(|a| a.text()).map(|t| t.len()).sum();
                if total_text_len > 0 && link_text_len as f32 / total_text_len as f32 > 0.8 && text.split_whitespace().count() < 10 {
                    continue;
                }
                lists_text.push(text);
            }
        }
        lists_text
    }
    
    fn extract_links(&self, document: &Html, base_url_str: &str) -> Result<Vec<LinkData>, Box<dyn std::error::Error>> {
        let link_selector = Selector::parse("a[href]").unwrap();
        let base = Url::parse(base_url_str)?;
    
        Ok(document
            .select(&link_selector)
            .filter_map(|element| {
                let href_attr = element.value().attr("href")?;
                if href_attr.trim().is_empty() { return None; }

                let text = element.text().collect::<String>().trim().to_string();
                
                let link_type = if href_attr.starts_with('#') {
                    LinkType::Anchor
                } else {
                    match base.join(href_attr) {
                        Ok(full_url) => {
                            if full_url.host_str() == Some(&self.domain) {
                                LinkType::Internal
                            } else {
                                LinkType::External
                            }
                        }
                        Err(_) => LinkType::External,
                    }
                };
                
                Some(LinkData {
                    text,
                    href: href_attr.to_string(),
                    link_type,
                })
            })
            .collect())
    }
    
    fn create_chunks(&self, full_text: &str, _headings: &[Heading], url: &str) -> Vec<TextChunk> {
        const CHUNK_SIZE: usize = 1000; 
        const OVERLAP: usize = 200;    

        let mut chunks = Vec::new();
        let mut current_byte_start = 0; 
        let text_len_bytes = full_text.len();
        let mut chunk_index = 0;

        if full_text.is_empty() {
            return chunks;
        }

        loop {
            while current_byte_start < text_len_bytes && !full_text.is_char_boundary(current_byte_start) {
                current_byte_start += 1;
            }

            if current_byte_start >= text_len_bytes {
                break;
            }

            let mut target_end_byte = (current_byte_start + CHUNK_SIZE).min(text_len_bytes);

            while target_end_byte < text_len_bytes && !full_text.is_char_boundary(target_end_byte) {
                target_end_byte += 1;
            }
            
            if target_end_byte <= current_byte_start && current_byte_start < text_len_bytes {
                if let Some((_idx, ch)) = full_text[current_byte_start..].char_indices().next() {
                    target_end_byte = current_byte_start + ch.len_utf8();
                } else { 
                    break; 
                }
            }
            target_end_byte = target_end_byte.min(text_len_bytes);
            
            if target_end_byte <= current_byte_start {
                break; 
            }

            let mut chunk_to_slice_end_byte = target_end_byte;

            if target_end_byte < text_len_bytes { 
                let mut sentence_search_limit = (target_end_byte + 100).min(text_len_bytes);
                while sentence_search_limit < text_len_bytes && !full_text.is_char_boundary(sentence_search_limit) {
                    sentence_search_limit += 1;
                }
                
                if sentence_search_limit > current_byte_start { 
                    let search_slice = &full_text[current_byte_start..sentence_search_limit];
                    if let Some(pos) = search_slice.rfind(". ") {
                        let sentence_end_abs_byte = current_byte_start + pos + 2; 
                        if sentence_end_abs_byte > current_byte_start && sentence_end_abs_byte <= sentence_search_limit {
                            if full_text.is_char_boundary(sentence_end_abs_byte) {
                                 chunk_to_slice_end_byte = sentence_end_abs_byte;
                            }
                        }
                    }
                }
            }
            
            if chunk_to_slice_end_byte <= current_byte_start {
                chunk_to_slice_end_byte = target_end_byte; 
                 if chunk_to_slice_end_byte <= current_byte_start { 
                     break; 
                 }
            }

            let chunk_text_slice = &full_text[current_byte_start..chunk_to_slice_end_byte];
            let trimmed_chunk_text = chunk_text_slice.trim();

            if !trimmed_chunk_text.is_empty() {
                chunks.push(TextChunk {
                    chunk_id: format!("{}#chunk{}", url, chunk_index),
                    text: trimmed_chunk_text.to_string(),
                    char_start: current_byte_start, 
                    char_end: chunk_to_slice_end_byte,
                    section_heading: None, 
                });
                chunk_index += 1;
            }

            let next_start_byte_candidate = chunk_to_slice_end_byte.saturating_sub(OVERLAP); // Removed mut

            if next_start_byte_candidate <= current_byte_start && chunk_to_slice_end_byte > current_byte_start {
                current_byte_start = chunk_to_slice_end_byte;
            } else if next_start_byte_candidate > current_byte_start {
                current_byte_start = next_start_byte_candidate;
            } else { 
                break;
            }
        }
        chunks
    }
    
    fn filter_url(&self, base_url_str: &str, href: &str) -> Option<String> {
        let lower_href = href.to_lowercase();
        let banned_extensions = [".pdf", ".jpg", ".jpeg", ".png", ".gif", ".zip", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx", ".mp3", ".mp4", ".avi", ".mov", ".xml", ".css", ".js", ".svg", ".webp", ".woff", ".woff2", ".ttf", ".eot", ".ics"];
        if banned_extensions.iter().any(|ext| lower_href.ends_with(ext) || lower_href.contains(&format!("{}?", ext)) ) {
            return None;
        }

        let banned_starts_patterns = ["#", "mailto:", "tel:", "javascript:", "data:"];
         for banned in &banned_starts_patterns {
            if lower_href.starts_with(banned) {
                return None;
            }
        }
        if href == "/cookies" || href == "/cookie-policy" {
             return None;
        }
        
        let base_url = match Url::parse(base_url_str) {
            Ok(url) => url,
            Err(_) => return None, 
        };

        match base_url.join(href) {
            Ok(mut full_url) => {
                if full_url.host_str() == Some(&self.domain) {
                    full_url.set_fragment(None);
                    let query_pairs: Vec<(String, String)> = full_url.query_pairs()
                        .filter(|(key, _)| !key.starts_with("utm_") && key != "fbclid" && key != "gclid")
                        .map(|(k, v)| (k.into_owned(), v.into_owned()))
                        .collect();
                    if query_pairs.is_empty() {
                        full_url.set_query(None);
                    } else {
                        let new_query = query_pairs.into_iter()
                            .map(|(k, v)| format!("{}={}", k, v))
                            .collect::<Vec<String>>()
                            .join("&");
                        full_url.set_query(Some(&new_query));
                    }
                    Some(full_url.to_string())
                } else {
                    None 
                }
            }
            Err(_) => None, 
        }
    }
    
    fn save_results(&self, root_url: &str) -> Result<(), Box<dyn std::error::Error>> {
        let output = CrawlOutput {
            domain: self.domain.clone(),
            root_url: root_url.to_string(),
            crawl_timestamp: Utc::now(),
            total_pages: self.pages.len(),
            pages: self.pages.clone(),
        };
        
        let output_dir = Path::new("crawled_data");
        fs::create_dir_all(&output_dir)?;
        
        let sanitized_domain = self.domain.replace(|c: char| !c.is_alphanumeric() && c != '-', "_");
        let filename = format!("{}/{}.json", output_dir.display(), sanitized_domain);
        let json = serde_json::to_string_pretty(&output)?;
        fs::write(&filename, json)?;
        
        println!("Saved {} pages to {}", self.pages.len(), filename);
        Ok(())
    }
}

fn main() {
    let root_url = "https://www.surrey.ac.uk/open-days";
    let max_depth = 2; 
    
    match Crawler::new(root_url) {
        Ok(mut crawler) => {
            crawler.crawl(root_url, 0, max_depth);
            
            if let Err(e) = crawler.save_results(root_url) {
                eprintln!("Error saving results: {}", e);
            } else if crawler.pages.is_empty() {
                println!("No pages were saved. The crawl might have resulted in no processable content or all pages were filtered out.");
            }
        }
        Err(e) => {
            eprintln!("Error initializing crawler: {}", e);
        }
    }
}



