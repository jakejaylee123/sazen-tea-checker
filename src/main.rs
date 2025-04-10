use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};

use reqwest::{self, Url};

use select::document::Document;
use select::predicate::{Attr, Class, Name, Predicate};

use std::collections::HashMap;
use std::env;
use std::fmt::{self, Write};
use std::time::Duration;

use url::Origin::Tuple;

#[derive(Debug, PartialEq, Eq, Clone)]
enum JobParameterError {
    NotPresent(&'static str),
    InvalidFormat(&'static str)
}

impl fmt::Display for JobParameterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JobParameterError::NotPresent(name) => write!(f, "Job parameter '{}' not found", name),
            JobParameterError::InvalidFormat(name) => write!(f, "Job parameter '{}' is invalid format", name)
        }
    }
}

struct JobParameters {
    interval_minutes: u64,

    products_url: String,
    matcha_brands: Vec<String>,

    smtp_url: String,
    smtp_user: String,
    smtp_password: String,
    smtp_transcipient: String,
    smtp_recipient: String,
    smtp_notification_subject: String
}

struct Product {
    url: String,
    code: String,
    name: String,
    maker: String,
    ingredients: String
}

const ENV_VAR_JOB_INTERVAL_MINUTES: &str = "JOB_INTERVAL_MINUTES";
const ENV_VAR_PRODUCTS_URL: &str = "PRODUCTS_URL";
const ENV_VAR_MATCHA_BRANDS: &str = "MATCHA_BRANDS";
const ENV_VAR_SMTP_URL: &str = "SMTP_URL";
const ENV_VAR_SMTP_USER: &str = "SMTP_USER";
const ENV_VAR_SMTP_PASSWORD: &str = "SMTP_PASSWORD";
const ENV_VAR_SMTP_TRANSCIPIENT: &str = "SMTP_TRANSCIPIENT";
const ENV_VAR_SMTP_RECIPIENT: &str = "SMTP_RECIPIENT";
const ENV_VAR_SMTP_NOTIFICATION_SUBJECT: &str = "SMTP_NOTIFICATION_SUBJECT";

const MATCHA_INGREDIENTS: [&str; 2] = ["matcha", "green tea powder"];

struct SazenTeaCheckerJob {
    parameters: JobParameters,
    client: reqwest::Client
}

impl SazenTeaCheckerJob {
    pub fn new(parameters: JobParameters) -> Self {
        Self { 
            parameters,
            client: reqwest::Client::new()
        }
    }

    async fn get_html_content(&self, url: &str) -> Result<String, String> {
        let result = self.client.get(url).send().await;
        let Ok(response) = result else {
            return Err(format!("GET request failed: {:?}", result.err()
                .ok_or_else(|| "Unknowm error")))
        };
    
        let text_result = response.text().await;
        let Ok(response_text) = text_result else {
            return Err(format!("Conversion of response to text failed: {:?}", text_result.err()
                .ok_or_else(|| "Unknown error")))
        };
        
        Ok(response_text)
    }

    async fn get_product_html_content(&self) -> Result<String, String> {
        self.get_html_content(&self.parameters.products_url).await
    }
    
    async fn get_product_list_from_html(&self, html: String) -> Result<Vec<Product>, String> {
        let products_document = Document::from(html.as_str());
        let product_elements = products_document.find(Class("product"));

        let product_url_result = Url::parse(&self.parameters.products_url);
        let Ok(product_url) = product_url_result else {
            return Err(format!("Error parsing product URL: {:?}", 
                product_url_result.err().ok_or_else(|| "Unknown error")))
        };

        let Tuple(scheme, product_base_url_parts, _) = product_url.origin() else {
            return Err(format!("The configured URL '{}' was not in the expected format", 
                self.parameters.products_url));
        };
        let product_base_url = format!("{}://{}", scheme, product_base_url_parts);
        
        let product_links: Vec<String> = product_elements
            .filter_map(|element| {
                let Some(link_element) = element
                    .find(Name("a")).nth(0) else { return None; };
                let Some(value) = link_element
                    .attr("href") else { return None; };

                let value = format!("{}{}", product_base_url, value);
                Some(value)
            })
            .collect();

        let mut products: Vec<Product> = vec![];
        for link in product_links {
            let product_detail_text_result = self.get_html_content(&link).await;
            let Ok(product_detail_text) = product_detail_text_result else {
                println!("Error retrieving product details: {:?}", 
                    product_detail_text_result.err().ok_or_else(|| "Unknown error"));
                continue;
            };

            let product_detail_document = Document::from(product_detail_text.as_str());

            let Some(product_name) = product_detail_document
                .find(Name("h1")
                .and(Attr("itemprop", "name"))).nth(0) else { continue; };
            let Some(product_info) = product_detail_document
                .find(Attr("id", "product-info")).nth(0) else { continue; };

            let mut product_info_hash: HashMap<String, String> = HashMap::new();
            let product_info_pairs: Vec<Vec<String>> = product_info.find(Name("p"))
                .map(|element| element.text().split(":")
                    .map(|part| part.trim().to_string()).collect())
                .collect();
            for (index, pair) in product_info_pairs.iter().enumerate() {
                product_info_hash.insert(
                    pair.iter().nth(0).unwrap_or(&format!("BAD_KEY_{}", index)).clone(), 
                    pair.iter().nth(1).unwrap_or(&format!("BAD_VALLUE_{}", index)).clone());
            }

            let product = Product {
                url: link,
                code: product_info_hash.get("Item code")
                    .unwrap_or(&"BAD_CODE".to_string()).clone(),
                name: product_name.text().clone(),
                maker: product_info_hash.get("Maker")
                    .unwrap_or(&"BAD_MAKER".to_string()).clone(),
                ingredients: product_info_hash.get("Ingredients")
                    .unwrap_or(&"BAD_INGREDIENTS".to_string()).clone()
            };

            println!("Found product '{}':\r\n - Item code: {}\r\n - Maker: {}\r\n - Ingredients: {}\r\n - URL: {}", 
                product.name, product.code, product.maker, product.ingredients, product.url);

            products.push(product);
        }
        
        Ok(products)
    }
    
    async fn get_matcha_product_list_from_html(&self, html: String) -> Result<Vec<Product>, String> {
        let product_list = self.get_product_list_from_html(html).await?;
        let matcha_product_list = product_list
            .into_iter()
            .filter(|product| {
                self.parameters.matcha_brands.iter().any(|brand| {
                    product.name.to_lowercase().contains(brand)
                        || product.maker.to_lowercase().contains(brand)
                })
            })
            .filter(|product| {
                MATCHA_INGREDIENTS.iter().any(|ingredient| 
                    product.ingredients.to_lowercase().contains(ingredient))
            })
            .collect();
        
        Ok(matcha_product_list)
    }
    
    fn send_product_listing_email(&self, products: &Vec<Product>) -> Result<String, String> {
        let parsed_smtp_transcipient_result = self.parameters.smtp_transcipient.parse(); 
        let Ok(parsed_smtp_transcipient) = parsed_smtp_transcipient_result else {
            return Err(format!("Error parsing transcipient email: {:?}", 
                parsed_smtp_transcipient_result.err().ok_or_else(|| "Unknowm error")))
        };

        let parsed_smtp_recipient_result = self.parameters.smtp_recipient.parse();
        let Ok(parsed_smtp_recipient) = parsed_smtp_recipient_result else {
            return Err(format!("Error parsing recipient email: {:?}", 
                parsed_smtp_recipient_result.err().ok_or_else(|| "Unknowm error")))
        };

        let message = Message::builder()
            .from(parsed_smtp_transcipient)
            .to(parsed_smtp_recipient)
            .subject(self.parameters.smtp_notification_subject.clone())
            .header(ContentType::TEXT_HTML);

        let mut body = String::new();
        write!(&mut body, "<p>Check out these matcha products!</p>\n\n").unwrap();
    
        write!(&mut body, "<ul>\n").unwrap();
        for product in products {
            write!(&mut body, "<li><strong>{} (Item code '{}')</strong>: {}\n",
                product.name, product.code, product.maker).unwrap();
            write!(&mut body, "<ul><li><a href=\"{}\">{}</a></li></ul>\n",
                product.url, product.url).unwrap();
        }
        write!(&mut body, "</ul>\n\n").unwrap();
    
        write!(&mut body, "<p>Have a great day!</p>").unwrap();
        
        let message_with_body_result = message.body(body);
        let Ok(message_with_body) = message_with_body_result else {
            return Err(format!("Error generating e-mail message: {:?}", 
                message_with_body_result.err().ok_or_else(|| "Unknown error")));
        };
    
        let creds = Credentials::new(
            self.parameters.smtp_user.to_owned(), 
            self.parameters.smtp_password.to_owned());
    
        // Open a remote connection to gmail
        let mailer = SmtpTransport::relay(&self.parameters.smtp_url)
            .expect("Unable to create mailer. Check SMTP URL...")
            .credentials(creds)
            .build();
    
        // Send the email
        match mailer.send(&message_with_body) {
            Ok(_) => Ok(format!("Email sent successfully!")),
            Err(e) => Err(format!("Could not send email: {e:?}"))
        }
    }
    
    async fn run_job_iteration(&self) -> Result<(), String> {
        let text_result = self.get_product_html_content().await?;
    
        let products = self.get_matcha_product_list_from_html(text_result).await?;
        if products.is_empty() {
            println!("No matcha products found in this check iteration.");
            return Ok(())
        }
    
        println!("Matcha products found... Sending e-mail...");
        self.send_product_listing_email(&products)?;
    
        Ok(())
    }
    
    pub async fn run_job_loop(&self) -> Result<(), String> {
        let interval_seconds = self.parameters.interval_minutes * 60;
    
        loop {
            println!("Running job iteration...");
            self.run_job_iteration().await?;

            println!("Sleeping for {} minutes...", self.parameters.interval_minutes);
            tokio::time::sleep(Duration::from_secs(interval_seconds)).await;
        }
    }
}

fn get_job_parameters() -> Result<JobParameters, JobParameterError> {
    Ok(JobParameters { 
        interval_minutes: env::var(ENV_VAR_JOB_INTERVAL_MINUTES)
            .map_err(|_| JobParameterError::NotPresent(ENV_VAR_JOB_INTERVAL_MINUTES))
            .and_then(|value| value.parse::<u64>()
                .map_err(|_| JobParameterError::InvalidFormat(ENV_VAR_JOB_INTERVAL_MINUTES)))?,
        products_url: env::var(ENV_VAR_PRODUCTS_URL)
            .map_err(|_| JobParameterError::NotPresent(ENV_VAR_PRODUCTS_URL))?,
        matcha_brands: env::var(ENV_VAR_MATCHA_BRANDS)
            .map_err(|_| JobParameterError::NotPresent(ENV_VAR_MATCHA_BRANDS))
            .map(|value| value.split(',').map(String::from).collect::<Vec<_>>())?,
        smtp_url: env::var(ENV_VAR_SMTP_URL)
            .map_err(|_| JobParameterError::NotPresent(ENV_VAR_SMTP_URL))?,
        smtp_user: env::var(ENV_VAR_SMTP_USER)
            .map_err(|_| JobParameterError::NotPresent(ENV_VAR_SMTP_USER))?,
        smtp_password: env::var(ENV_VAR_SMTP_PASSWORD)
            .map_err(|_| JobParameterError::NotPresent(ENV_VAR_SMTP_PASSWORD))?,
        smtp_transcipient: env::var(ENV_VAR_SMTP_TRANSCIPIENT)
            .map_err(|_| JobParameterError::NotPresent(ENV_VAR_SMTP_TRANSCIPIENT))?,
        smtp_recipient: env::var(ENV_VAR_SMTP_RECIPIENT)
            .map_err(|_| JobParameterError::NotPresent(ENV_VAR_SMTP_RECIPIENT))?,
        smtp_notification_subject: env::var(ENV_VAR_SMTP_NOTIFICATION_SUBJECT)
            .map_err(|_| JobParameterError::NotPresent(ENV_VAR_SMTP_NOTIFICATION_SUBJECT))?,
    })
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let parameters = get_job_parameters();
    match parameters {
        Err(error) => println!("Error getting parameters: {}", error),
        Ok(parameters) => {
            let job = SazenTeaCheckerJob::new(parameters);
            job.run_job_loop().await?;
        }
    }

    Ok(())
}
