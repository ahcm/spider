use crate::black_list::contains;
use crate::configuration::Configuration;
use crate::page::Page;
use crate::utils::{Client};

use rayon::ThreadPool;
use rayon::ThreadPoolBuilder;
use robotparser_fork::RobotFileParser;

use hashbrown::HashSet;
use std::{sync, thread, time::Duration};
use reqwest::header::CONNECTION;
use reqwest::header;
use sync::mpsc::{channel, Sender, Receiver};

/// Represent a website to scrawl. To start crawling, instanciate a new `struct` using
/// <pre>
/// let mut localhost = Website::new("http://example.com");
/// localhost.crawl();
/// </pre>
/// `Website` will be filled with `Pages` when crawled. To get them, just use
/// <pre>
/// for page in localhost.get_pages() {
///     // do something
/// }
/// </pre>
#[derive(Debug)]
pub struct Website<'a> {
    // configuration properies
    pub configuration: Configuration,
    /// this is a start URL given when instanciate with `new`
    pub domain: String,
    /// contains all non-visited URL
    links: HashSet<String>,
    /// contains all visited URL
    links_visited: HashSet<String>,
    /// contains page visited
    pages: Vec<Page>,
    /// callback when a link is found
    pub on_link_find_callback: fn(String) -> String,
    /// Robot.txt parser holder
    robot_file_parser: RobotFileParser<'a>,
    // ignore holding page in memory, pages will always be empty
    pub page_store_ignore: bool,
    client: Client,
    configured: bool,
    pool: ThreadPool,
}

type Message = (Page, HashSet<String>);

impl<'a> Website<'a> {
    /// Initialize Website object with a start link to scrawl.
    pub fn new(domain: &str) -> Self {
        let config = Configuration::new();
        let default_threads = config.concurrency;

        Self {
            configuration: config,
            links_visited: HashSet::new(),
            pages: Vec::new(),
            robot_file_parser: RobotFileParser::new(&format!("{}/robots.txt", domain)), // TODO: lazy establish
            links: HashSet::from([format!("{}/", domain)]),
            on_link_find_callback: |s| s,
            page_store_ignore: false,
            domain: domain.to_owned(),
            client: Default::default(),
            configured: false,
            pool: ThreadPoolBuilder::new().num_threads(default_threads).build().unwrap(),
        }
    }

    /// page getter
    pub fn get_pages(&self) -> &Vec<Page> {
        &self.pages
    }

    /// crawl delay getter
    pub fn get_delay(&self) -> Duration {
        Duration::from_millis(self.configuration.delay)
    }

    /// configure the robots parser on initial crawl attempt and run
    pub fn configure_robots_parser(&mut self) {
        if self.configuration.respect_robots_txt && self.robot_file_parser.mtime() == 0 {
            self.robot_file_parser.user_agent = self.configuration.user_agent.to_string();
            self.robot_file_parser.read();
            self.configuration.delay = self
                .robot_file_parser
                .get_crawl_delay(&self.robot_file_parser.user_agent) // returns the crawl delay in seconds
                .unwrap_or(self.get_delay())
                .as_millis() as u64;
        }
    }

    /// configure http client
    fn configure_http_client(&mut self, user_agent: Option<String>) {
        let mut headers = header::HeaderMap::new();
        headers.insert(CONNECTION, header::HeaderValue::from_static("keep-alive"));

        self.client = Client::builder()
            .default_headers(headers)
            .user_agent(user_agent.unwrap_or(self.configuration.user_agent.to_string()))
            .build()
            .expect("Failed building client.")
    }

    /// configure rayon thread pool
    fn create_thread_pool(&mut self) {
        if self.configuration.concurrency != self.pool.current_num_threads() {
            self.pool = ThreadPoolBuilder::new()
                .num_threads(self.configuration.concurrency)
                .build()
                .expect("Failed building thread pool.")
        }
    }

    /// Recursive walk the website
    pub fn walk(&mut self, link: &String, tx: &Sender<Message>) {
        if !self.is_allowed(&link) {
            return
        }

        self.log(&format!("- fetch {}", &link));
        self.links_visited.insert(link.into());
        let on_link_find_callback = self.on_link_find_callback;
        
        if self.configuration.delay > 0 {
            thread::sleep(self.get_delay());
        }

        let link = link.clone();
        let tx = tx.clone();
        let cx = self.client.clone();

        self.pool.spawn(move || {
            let link_result = on_link_find_callback(link);
            let mut page = Page::new(&link_result, &cx);
            let links = page.links();

            tx.send((page, links)).unwrap();
        });
    }

    pub fn configure(&mut self ) {
        if !self.configured {
            self.configure_robots_parser();
            self.configure_http_client(None);
            self.create_thread_pool();
            self.configured = true;
        }
    }

    /// Start to crawl website
    pub fn crawl(&mut self, link: Option<String>) {
        self.configure();
        let (tx, rx): (Sender<Message>, Receiver<Message>) = channel();

        self.walk(&link.unwrap_or(self.links.get(&format!("{}/", self.domain)).unwrap_or(&self.domain).to_string()), &tx);

        drop(tx);

        let mut new_links: HashSet<String> = HashSet::new();
        
        rx.into_iter().for_each(|page| {
            let (page, links) = page;
            self.log(&format!("- parse {}", &page.get_url()));
            new_links.extend(links);
            
            if !self.page_store_ignore {
                self.pages.push(page);
            }
            
        });
        
        let new_links = &new_links - &self.links_visited;
        
        new_links.into_iter().for_each(|link| {
            self.crawl(Some(link));
        });
    }

    /// return `true` if URL:
    ///
    /// - is not already crawled
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)  
    pub fn is_allowed(&self, link: &String) -> bool {
        if self.links_visited.contains(link) {
            return false;
        }
        if contains(&self.configuration.blacklist_url, link) {
            return false;
        }
        if self.configuration.respect_robots_txt && !self.robot_file_parser.can_fetch("*", link) {
            return false;
        }

        true
    }

    /// log to console if configuration verbose
    pub fn log(&self, message: impl AsRef<str>) {
        if self.configuration.verbose {
            println!("{}", message.as_ref());
        }
    }
}

impl<'a> Drop for Website<'a> {
    fn drop(&mut self) {}
}

#[test]
fn crawl() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.crawl(None);
    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".to_string()),
        "{:?}",
        website.links_visited
    );
}

#[test]
fn crawl_invalid() {
    let url = "https://w.com";
    let mut website: Website = Website::new(url);
    website.crawl(None);
    let mut uniq = HashSet::new();
    uniq.insert(format!("{}/", url.to_string())); // TODO: remove trailing slash mutate

    assert_eq!(website.links_visited, uniq); // only the target url should exist
}

#[test]
fn crawl_link_callback() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.on_link_find_callback = |s| {
        println!("callback link target: {}", s);
        s
    };
    website.crawl(None);
    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".to_string()),
        "{:?}",
        website.links_visited
    );
}

#[test]
fn not_crawl_blacklist() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website
        .configuration
        .blacklist_url
        .push("https://choosealicense.com/licenses/".to_string());
    website.crawl(None);
    assert!(
        !website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".to_string()),
        "{:?}",
        website.links_visited
    );
}

#[test]
#[cfg(feature = "regex")]
fn not_crawl_blacklist_regex() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website
        .configuration
        .blacklist_url
        .push("/choosealicense.com/".to_string());
    website.crawl();
    assert_eq!(website.links_visited.len(), 0);
}

#[test]
fn test_respect_robots_txt() {
    let mut website: Website = Website::new("https://stackoverflow.com");
    website.configuration.respect_robots_txt = true;
    assert_eq!(website.configuration.delay, 250);
    assert!(!website.is_allowed(&"https://stackoverflow.com/posts/".to_string()));

    // test match for bing bot
    let mut website_second: Website = Website::new("https://www.mongodb.com");
    website_second.configuration.respect_robots_txt = true;
    website_second.configuration.user_agent = "bingbot";
    website_second.configure_robots_parser();
    assert_eq!(
        website_second.configuration.user_agent,
        website_second.robot_file_parser.user_agent
    );
    assert_eq!(website_second.configuration.delay, 60000); // should equal one minute in ms

    // test crawl delay with wildcard agent [DOES not work when using set agent]
    let mut website_third: Website = Website::new("https://www.mongodb.com");
    website_third.configuration.respect_robots_txt = true;
    website_third.configure_robots_parser();

    assert_eq!(website_third.configuration.delay, 10000); // should equal 10 seconds in ms
}

#[test]
fn test_link_duplicates() {
    fn has_unique_elements<T>(iter: T) -> bool
    where
        T: IntoIterator,
        T::Item: Eq + std::hash::Hash,
    {
        let mut uniq = HashSet::new();
        iter.into_iter().all(move |x| uniq.insert(x))
    }

    let mut website: Website = Website::new("http://0.0.0.0:8000");
    website.crawl(None);

    assert!(has_unique_elements(&website.links_visited));
}
