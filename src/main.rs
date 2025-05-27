use scraper::{Html, Selector};
use url::{Url, ParseError};
use std::collections::HashSet;


struct CrawlResult {
    visited_urls: Vec<String>,
    all_headings: Vec<Vec<String>>,
    all_paragraphs: Vec<Vec<String>>,
}

fn crawl(url: &str, 
        visited: &mut HashSet<String>, 
        depth: usize, 
        max_depth: usize) -> CrawlResult {

    // Base case
    if depth >= max_depth || visited.contains(url) {
        return CrawlResult {
            visited_urls: vec![],
            all_headings: vec![],
            all_paragraphs: vec![],
        };
    }
    // Mark as visited
    visited.insert(url.to_string());

    // Scrape the page
    let page_data = match scrape(url) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Error scraping {}: {}", url, e);
            return CrawlResult {
                visited_urls: vec![url.to_string()],
                all_headings: vec![],
                all_paragraphs: vec![],
            }
        }
    };

    let mut result = CrawlResult {
        visited_urls: vec![url.to_string()],
        all_headings: vec![page_data.headings],  // Note: vec of vec!
        all_paragraphs: vec![page_data.paragraphs],
    };
    
    for link in page_data.links.iter().take(30) { // Only use 30 links per page
        if let Some(filtered_url) = filter_links(url, link) {
            if !visited.contains(&filtered_url) {
                println!("Crawling: {} (depth: {})", filtered_url, depth + 1);
                let sub_result = crawl(
                    &filtered_url, 
                    visited, 
                    depth + 1, 
                    max_depth
                );
                // Combine results
                result.visited_urls.extend(sub_result.visited_urls);
                result.all_headings.extend(sub_result.all_headings);
                result.all_paragraphs.extend(sub_result.all_paragraphs);
            }
        }
    }
    result
}

#[derive(Debug)]
struct PageData {
    headings: Vec<String>,
    paragraphs: Vec<String>,
    links: Vec<Link>,
}

#[derive(Debug)]
struct Link {
    text: String,
    href: String,
}

fn filter_links(base_url: &str, link: &Link) -> Option<String>{
    let banned_starts = vec!["/cookies", "../", "#", "?"]; // mailto, tel can be added

    for bs in banned_starts {
        if link.href.starts_with(bs) {
            println!("Exluding link: {}", link.href);
            return None;
        } 
    }

    if link.href.starts_with("http"){
        return Some(link.href.clone());

    }
    else {
        let base = Url::parse(base_url).ok()?;
        let full_url = base.join(&link.href).ok();
        return full_url.map(|url| url.to_string());
    }
}

fn scrape(url: &str) -> Result<PageData, Box<dyn std::error::Error>>{
    let response = reqwest::blocking::get(url)?;
    let body = response.text()?;

    let document = Html::parse_document(&body);
    let heading_tags: Vec<&str> = vec!["h1", "h2", "h3", "h4", "h5", "h6"];
    let p_selector = Selector::parse("p").unwrap();
    
    let mut headings: Vec<String> = Vec::new();

    for h_tag in heading_tags{
        let h_selector = Selector::parse(h_tag).unwrap();
        let h_elements: Vec<String> = document
            .select(&h_selector)
            .map(|element| element.text().collect::<String>())
            .collect();
        headings.extend(h_elements);
    }
    let paragraphs: Vec<String> = document
        .select(&p_selector)
        .map(|element| element.text().collect::<String>())
        .collect();
    
    let link_selector = Selector::parse("a").unwrap();
    let links: Vec<Link> = document
        .select(&link_selector)
        .filter_map(|element| {Some(Link{
            text: element.text().collect::<String>(),
            href: element.value().attr("href")?.to_string()})
        }).collect();

    Ok(PageData{
        headings: headings,
        paragraphs: paragraphs,
        links: links
    })
}
fn main() {
    let url = "https://www.surrey.ac.uk/open-days";
    
    let mut visited = HashSet::new();
    let mut res = crawl("https://www.surrey.ac.uk/open-days", &mut visited, 0, 3);
    println!("{:?}", res.all_headings); // save all important data as json instead
    let res = crawl("https://www.surrey.ac.uk/open-days", &mut visited, 0, 3);
    println!("{:?}", res.all_headings);

    //     match page {
    //         Ok(page_data) => {
    //             println!("Worked");
    //             println!("headings: {:?}", page_data.headings);
    //             // println!("links: {:?}", page_data.links);
    //             // println!("paragraphs: {:?}", page_data.paragraphs);
    //         }
    //         Err(error_message) => eprintln!("Uh-oh: {}", error_message)
    //     }
    //     println!("Web crawler in Rust!");

}
