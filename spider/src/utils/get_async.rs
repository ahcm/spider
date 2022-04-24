use crate::reqwest::{Client};
use reqwest::StatusCode;

pub async fn fetch_page_html_async(url: &str, client: &Client) -> String {
    let mut body = String::new();

    // silence errors for top level logging
    match client.get(url).send().await {
        Ok(res) if res.status() == StatusCode::OK => match res.text().await {
            Ok(text) => body = text,
            Err(_) => {},
        },
        Ok(_) => (),
        Err(_) => {}
    }

    body
}
