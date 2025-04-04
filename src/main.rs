use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};

use reqwest;

use select::document::Document;
use select::predicate::{Class, Name};

use std::env;
use std::fmt::{self, Write};
use std::time::Duration;

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
    name: String,
    description: String
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

const MATCHA_VARIANTS: [&str; 5] = [
    "usucha",
    "koicha",
    "shiro",
    "mukashi",
    "matcha"
];

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

    async fn get_product_html_content(&self) -> Result<String, String> {
        let result = self.client.get(&self.parameters.products_url).send().await;
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
    
    fn get_product_list_from_html(&self, html: String) -> Result<Vec<Product>, String> {
        let document = Document::from(html.as_str() );
        let product_elements = document.find(Class("product"));
    
        let mut product_list: Vec<Product> = vec![];
        for element in product_elements {
            let Some(name_element) = element.find(Class("product-name")).nth(0) else {
                continue;
            };

            let Some(description_element) = element.find(Name("p")).nth(0) else {
                continue;
            };

            product_list.push(Product {
                name: name_element.text().clone(),
                description: description_element.text().clone()
            });
        }

        Ok(product_list)
    }
    
    fn get_matcha_product_list_from_html(&self, html: String) -> Result<Vec<Product>, String> {
        Ok(self.get_product_list_from_html(html)?
            .into_iter()
            .filter(|product| {
                self.parameters.matcha_brands.iter().any(|brand| {
                    product.name.to_lowercase().contains(brand)
                        || product.description.to_lowercase().contains(brand)
                })
            })
            .filter(|product| {
                MATCHA_VARIANTS.iter().any(|variant| {
                    product.name.to_lowercase().contains(variant)
                        || product.description.to_lowercase().contains(variant)
                })
            })
            .collect())
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
            write!(&mut body, "<li><strong>{}</strong>: {}\n", product.name, product.description).unwrap();
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
    
        let products = self.get_matcha_product_list_from_html(text_result)?;
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
