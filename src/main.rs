use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};

use reqwest;

use select::document::Document;
use select::predicate::{Class, Name};

use std::env;
use std::fmt;
use std::thread;
use std::time::Duration;

#[derive(Debug, PartialEq, Eq, Clone)]
enum JobParameterError {
    NotPresent,
    InvalidFormat
}

impl fmt::Display for JobParameterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            JobParameterError::NotPresent => write!(f, "Job parameter (environment variable) not found"),
            JobParameterError::InvalidFormat => write!(f, "Job parameter was not in valid format")
        }
    }
}

struct JobParameters {
    interval_minutes: u64,

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
const ENV_VAR_SMTP_URL: &str = "SMTP_URL";
const ENV_VAR_SMTP_USER: &str = "SMTP_USER";
const ENV_VAR_SMTP_PASSWORD: &str = "SMTP_PASSWORD";
const ENV_VAR_SMTP_TRANSCIPIENT: &str = "SMTP_TRANSCIPIENT";
const ENV_VAR_SMTP_RECIPIENT: &str = "SMTP_RECIPIENT";
const ENV_VAR_SMTP_NOTIFICATION_SUBJECT: &str = "SMTP_NOTIFICATION_SUBJECT";

const SAZEN_TEA_PRODUCTS_URL: &str = "https://www.sazentea.com/en/products";

const MATCHA_BRANDS: [&str; 4] = [
    "marukyu koyamaen",
    "horii shichimeien",
    "kanbayashi shunsho",
    "maruyasu"
];
const MATCHA_VARIANTS: [&str; 5] = [
    "usucha",
    "koicha",
    "shiro",
    "mukashi",
    "matcha"
];

struct SazenTeaCheckerJob {
    parameters: JobParameters
}

impl SazenTeaCheckerJob {
    pub const fn new(parameters: JobParameters) -> Self {
        Self { parameters }
    }

    async fn get_product_html_content(&self) -> Result<String, String> {
        let result = reqwest::get(SAZEN_TEA_PRODUCTS_URL).await;
        let Ok(response) = result else {
            return Err(format!("GET request failed: {:?}", result.err().unwrap()))
        };
    
        let text_result = response.text().await;
        let Ok(response_text) = text_result else {
            return Err(format!("Conversion of response to text failed: {:?}", text_result.err()))
        };
        
        Ok(response_text)
    }
    
    fn get_product_list_from_html(&self, html: String) -> Vec<Product> {
        let document = Document::from(html.as_str() );
        let product_elements = document.find(Class("product"));
    
        product_elements
            .into_iter()
            .map(|element| Product { 
                name: element.find(Class("product-name")).nth(0).unwrap().text().clone(),
                description: element.find(Name("p")).nth(0).unwrap().text().clone()
            })
            .collect()
    }
    
    fn get_matcha_product_list_from_html(&self, html: String) -> Vec<Product> {
        self.get_product_list_from_html(html)
            .into_iter()
            .filter(|product| {
                MATCHA_BRANDS.iter().any(|brand| {
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
            .collect()
    }
    
    fn send_product_listing_email(&self, products: &Vec<Product>) -> Result<String, String> {
        let mut message_body_builder = String::new();
        message_body_builder.push_str("<p>Check out these matcha products!</p>\n\n");
    
        message_body_builder.push_str("<ul>\n");
        for product in products {
            message_body_builder
                .push_str(format!("<li><strong>{0}</strong>: {1}\n", product.name, product.description).as_str());
        }
        message_body_builder.push_str("</ul>\n\n");
    
        message_body_builder.push_str("<p>Have a great day!</p>");
    
        let email = Message::builder()
            .from(self.parameters.smtp_transcipient.parse().unwrap())
            .to(self.parameters.smtp_recipient.parse().unwrap())
            .subject(self.parameters.smtp_notification_subject.clone())
            .header(ContentType::TEXT_HTML)
            .body(message_body_builder)
            .unwrap();
    
        let creds = Credentials::new(
            self.parameters.smtp_user.to_owned(), 
            self.parameters.smtp_password.to_owned());
    
        // Open a remote connection to gmail
        let mailer = SmtpTransport::relay(&self.parameters.smtp_url)
            .unwrap()
            .credentials(creds)
            .build();
    
        // Send the email
        match mailer.send(&email) {
            Ok(_) => Ok(format!("Email sent successfully!")),
            Err(e) => Err(format!("Could not send email: {e:?}"))
        }
    }
    
    async fn run_job_iteration(&self) -> Result<(), String> {
        let text_result = self.get_product_html_content().await?;
    
        let products = self.get_matcha_product_list_from_html(text_result);
        if products.iter().count() <= 0 {
            println!("No matcha products found in this check iteration.");
            return Ok(())
        }
    
        self.send_product_listing_email(&products)?;
    
        Ok(())
    }
    
    pub async fn run_job_loop(&self) -> Result<(), String> {
        let interval_seconds = self.parameters.interval_minutes * 60;
    
        loop {
            println!("Running job iteration...");
            self.run_job_iteration().await?;

            println!("Sleeping for {:?} minutes...", self.parameters.interval_minutes);
            thread::sleep(Duration::from_secs(interval_seconds));
        }
    }
}


fn get_job_parameters() -> Result<JobParameters, JobParameterError> {
    Ok(JobParameters { 
        interval_minutes: env::var(ENV_VAR_JOB_INTERVAL_MINUTES)
            .map_err(|_| JobParameterError::NotPresent)
            .and_then(|value| value.parse::<u64>()
                .map_err(|_| JobParameterError::InvalidFormat))?,
        smtp_url: env::var(ENV_VAR_SMTP_URL)
            .map_err(|_| JobParameterError::NotPresent)?,
        smtp_user: env::var(ENV_VAR_SMTP_USER)
            .map_err(|_| JobParameterError::NotPresent)?,
        smtp_password: env::var(ENV_VAR_SMTP_PASSWORD)
            .map_err(|_| JobParameterError::NotPresent)?,
        smtp_transcipient: env::var(ENV_VAR_SMTP_TRANSCIPIENT)
            .map_err(|_| JobParameterError::NotPresent)?,
        smtp_recipient: env::var(ENV_VAR_SMTP_RECIPIENT)
            .map_err(|_| JobParameterError::NotPresent)?,
        smtp_notification_subject: env::var(ENV_VAR_SMTP_NOTIFICATION_SUBJECT)
            .map_err(|_| JobParameterError::NotPresent)?,
    })
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let parameters = get_job_parameters();
    match parameters {
        Err(error) => Err(format!("Error getting parameters: {:?}", error)),
        Ok(parameters) => {
            let job = SazenTeaCheckerJob::new(parameters);
            job.run_job_loop().await?;

            Ok(())
        }
    }
}
