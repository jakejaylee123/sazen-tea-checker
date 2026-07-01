package main

import (
	"fmt"
	"io"
	"log"
	"net/http"
	"net/mail"
	"net/smtp"
	"net/url"
	"os"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/PuerkitoBio/goquery"
)

// JobParameterError mirrors the two failure modes when reading job parameters
// from the environment: a variable is missing, or it is present but malformed.
type JobParameterError struct {
	Kind ParameterErrorKind
	Name string
}

type ParameterErrorKind int

const (
	ErrNotPresent ParameterErrorKind = iota
	ErrInvalidFormat
)

func (e *JobParameterError) Error() string {
	switch e.Kind {
	case ErrNotPresent:
		return fmt.Sprintf("Job parameter '%s' not found", e.Name)
	case ErrInvalidFormat:
		return fmt.Sprintf("Job parameter '%s' is invalid format", e.Name)
	default:
		return fmt.Sprintf("Job parameter '%s' error", e.Name)
	}
}

type JobParameters struct {
	IntervalMinutes uint64

	ProductsURL  string
	MatchaBrands []string

	SMTPURL                 string
	SMTPUser                string
	SMTPPassword            string
	SMTPTranscipient        string
	SMTPRecipient           string
	SMTPNotificationSubject string
}

type Product struct {
	URL         string
	Code        string
	Name        string
	Maker       string
	Ingredients string
}

const (
	envJobIntervalMinutes      = "JOB_INTERVAL_MINUTES"
	envProductsURL             = "PRODUCTS_URL"
	envMatchaBrands            = "MATCHA_BRANDS"
	envSMTPURL                 = "SMTP_URL"
	envSMTPUser                = "SMTP_USER"
	envSMTPPassword            = "SMTP_PASSWORD"
	envSMTPTranscipient        = "SMTP_TRANSCIPIENT"
	envSMTPRecipient           = "SMTP_RECIPIENT"
	envSMTPNotificationSubject = "SMTP_NOTIFICATION_SUBJECT"
)

var matchaIngredients = []string{"matcha", "green tea powder"}

type SazenTeaCheckerJob struct {
	parameters JobParameters
	client     *http.Client
}

func NewSazenTeaCheckerJob(parameters JobParameters) *SazenTeaCheckerJob {
	return &SazenTeaCheckerJob{
		parameters: parameters,
		client:     &http.Client{},
	}
}

func (j *SazenTeaCheckerJob) getHTMLContent(url string) (string, error) {
	response, err := j.client.Get(url)
	if err != nil {
		return "", fmt.Errorf("GET request failed: %w", err)
	}
	defer response.Body.Close()

	body, err := io.ReadAll(response.Body)
	if err != nil {
		return "", fmt.Errorf("Conversion of response to text failed: %w", err)
	}

	return string(body), nil
}

func (j *SazenTeaCheckerJob) getProductHTMLContent() (string, error) {
	return j.getHTMLContent(j.parameters.ProductsURL)
}

func (j *SazenTeaCheckerJob) getProductListFromHTML(html string) ([]Product, error) {
	productsDocument, err := goquery.NewDocumentFromReader(strings.NewReader(html))
	if err != nil {
		return nil, fmt.Errorf("Error parsing products HTML: %w", err)
	}

	productURL, err := url.Parse(j.parameters.ProductsURL)
	if err != nil {
		return nil, fmt.Errorf("Error parsing product URL: %w", err)
	}
	if productURL.Scheme == "" || productURL.Hostname() == "" {
		return nil, fmt.Errorf("The configured URL '%s' was not in the expected format", j.parameters.ProductsURL)
	}
	productBaseURL := fmt.Sprintf("%s://%s", productURL.Scheme, productURL.Hostname())

	var productLinks []string
	productsDocument.Find(".product").Each(func(_ int, element *goquery.Selection) {
		href, exists := element.Find("a").First().Attr("href")
		if !exists {
			return
		}
		productLinks = append(productLinks, productBaseURL+href)
	})

	products := []Product{}
	for _, link := range productLinks {
		productDetailText, err := j.getHTMLContent(link)
		if err != nil {
			fmt.Printf("Error retrieving product details: %v\n", err)
			continue
		}

		productDetailDocument, err := goquery.NewDocumentFromReader(strings.NewReader(productDetailText))
		if err != nil {
			fmt.Printf("Error parsing product details: %v\n", err)
			continue
		}

		productName := productDetailDocument.Find("h1[itemprop='name']").First()
		if productName.Length() == 0 {
			continue
		}
		productInfo := productDetailDocument.Find("#product-info").First()
		if productInfo.Length() == 0 {
			continue
		}

		productInfoHash := make(map[string]string)
		productInfo.Find("p").Each(func(index int, element *goquery.Selection) {
			parts := strings.Split(element.Text(), ":")
			key := fmt.Sprintf("BAD_KEY_%d", index)
			value := fmt.Sprintf("BAD_VALLUE_%d", index)
			if len(parts) > 0 {
				key = strings.TrimSpace(parts[0])
			}
			if len(parts) > 1 {
				value = strings.TrimSpace(parts[1])
			}
			productInfoHash[key] = value
		})

		product := Product{
			URL:         link,
			Code:        getOrDefault(productInfoHash, "Item code", "BAD_CODE"),
			Name:        strings.TrimSpace(productName.Text()),
			Maker:       getOrDefault(productInfoHash, "Maker", "BAD_MAKER"),
			Ingredients: getOrDefault(productInfoHash, "Ingredients", "BAD_INGREDIENTS"),
		}

		fmt.Printf("Found product '%s':\r\n - Item code: %s\r\n - Maker: %s\r\n - Ingredients: %s\r\n - URL: %s\n",
			product.Name, product.Code, product.Maker, product.Ingredients, product.URL)

		products = append(products, product)
	}

	return products, nil
}

func (j *SazenTeaCheckerJob) getMatchaProductListFromHTML(html string) ([]Product, error) {
	productList, err := j.getProductListFromHTML(html)
	if err != nil {
		return nil, err
	}

	matchaProductList := []Product{}
	for _, product := range productList {
		nameLower := strings.ToLower(product.Name)
		makerLower := strings.ToLower(product.Maker)
		ingredientsLower := strings.ToLower(product.Ingredients)

		brandMatch := slices.ContainsFunc(j.parameters.MatchaBrands, func(brand string) bool {
			return strings.Contains(nameLower, brand) || strings.Contains(makerLower, brand)
		})
		ingredientMatch := slices.ContainsFunc(matchaIngredients, func(ingredient string) bool {
			return strings.Contains(ingredientsLower, ingredient)
		})

		if brandMatch && ingredientMatch {
			matchaProductList = append(matchaProductList, product)
		}
	}

	return matchaProductList, nil
}

func (j *SazenTeaCheckerJob) sendProductListingEmail(products []Product) (string, error) {
	from, err := mail.ParseAddress(j.parameters.SMTPTranscipient)
	if err != nil {
		return "", fmt.Errorf("Error parsing transcipient email: %w", err)
	}
	to, err := mail.ParseAddress(j.parameters.SMTPRecipient)
	if err != nil {
		return "", fmt.Errorf("Error parsing recipient email: %w", err)
	}

	var body strings.Builder
	fmt.Fprintf(&body, "<p>Check out these matcha products!</p>\n\n")
	fmt.Fprintf(&body, "<ul>\n")
	for _, product := range products {
		fmt.Fprintf(&body, "<li><strong>%s (Item code '%s')</strong>: %s\n",
			product.Name, product.Code, product.Maker)
		fmt.Fprintf(&body, "<ul><li><a href=\"%s\">%s</a></li></ul>\n",
			product.URL, product.URL)
	}
	fmt.Fprintf(&body, "</ul>\n\n")
	fmt.Fprintf(&body, "<p>Have a great day!</p>")

	message := buildMIMEMessage(
		j.parameters.SMTPTranscipient,
		j.parameters.SMTPRecipient,
		j.parameters.SMTPNotificationSubject,
		body.String(),
	)

	auth := smtp.PlainAuth("", j.parameters.SMTPUser, j.parameters.SMTPPassword, j.parameters.SMTPURL)
	addr := fmt.Sprintf("%s:587", j.parameters.SMTPURL)

	if err := smtp.SendMail(addr, auth, from.Address, []string{to.Address}, []byte(message)); err != nil {
		return "", fmt.Errorf("Could not send email: %w", err)
	}

	return "Email sent successfully!", nil
}

func buildMIMEMessage(from, to, subject, htmlBody string) string {
	var msg strings.Builder
	fmt.Fprintf(&msg, "From: %s\r\n", from)
	fmt.Fprintf(&msg, "To: %s\r\n", to)
	fmt.Fprintf(&msg, "Subject: %s\r\n", subject)
	fmt.Fprintf(&msg, "MIME-Version: 1.0\r\n")
	fmt.Fprintf(&msg, "Content-Type: text/html; charset=\"UTF-8\"\r\n")
	fmt.Fprintf(&msg, "\r\n")
	msg.WriteString(htmlBody)
	return msg.String()
}

func (j *SazenTeaCheckerJob) runJobIteration() error {
	text, err := j.getProductHTMLContent()
	if err != nil {
		return err
	}

	products, err := j.getMatchaProductListFromHTML(text)
	if err != nil {
		return err
	}
	if len(products) == 0 {
		fmt.Println("No matcha products found in this check iteration.")
		return nil
	}

	fmt.Println("Matcha products found... Sending e-mail...")
	if _, err := j.sendProductListingEmail(products); err != nil {
		return err
	}

	return nil
}

func (j *SazenTeaCheckerJob) runJobLoop() error {
	interval := time.Duration(j.parameters.IntervalMinutes) * time.Minute

	for {
		fmt.Println("Running job iteration...")
		if err := j.runJobIteration(); err != nil {
			return err
		}

		fmt.Printf("Sleeping for %d minutes...\n", j.parameters.IntervalMinutes)
		time.Sleep(interval)
	}
}

func getOrDefault(m map[string]string, key, fallback string) string {
	if value, ok := m[key]; ok {
		return value
	}
	return fallback
}

func requireEnv(name string) (string, error) {
	value, ok := os.LookupEnv(name)
	if !ok {
		return "", &JobParameterError{Kind: ErrNotPresent, Name: name}
	}
	return value, nil
}

func getJobParameters() (*JobParameters, error) {
	intervalRaw, err := requireEnv(envJobIntervalMinutes)
	if err != nil {
		return nil, err
	}
	intervalMinutes, parseErr := strconv.ParseUint(intervalRaw, 10, 64)
	if parseErr != nil {
		return nil, &JobParameterError{Kind: ErrInvalidFormat, Name: envJobIntervalMinutes}
	}

	productsURL, err := requireEnv(envProductsURL)
	if err != nil {
		return nil, err
	}

	matchaBrandsRaw, err := requireEnv(envMatchaBrands)
	if err != nil {
		return nil, err
	}
	matchaBrands := strings.Split(matchaBrandsRaw, ",")

	smtpURL, err := requireEnv(envSMTPURL)
	if err != nil {
		return nil, err
	}
	smtpUser, err := requireEnv(envSMTPUser)
	if err != nil {
		return nil, err
	}
	smtpPassword, err := requireEnv(envSMTPPassword)
	if err != nil {
		return nil, err
	}
	smtpTranscipient, err := requireEnv(envSMTPTranscipient)
	if err != nil {
		return nil, err
	}
	smtpRecipient, err := requireEnv(envSMTPRecipient)
	if err != nil {
		return nil, err
	}
	smtpNotificationSubject, err := requireEnv(envSMTPNotificationSubject)
	if err != nil {
		return nil, err
	}

	return &JobParameters{
		IntervalMinutes:         intervalMinutes,
		ProductsURL:             productsURL,
		MatchaBrands:            matchaBrands,
		SMTPURL:                 smtpURL,
		SMTPUser:                smtpUser,
		SMTPPassword:            smtpPassword,
		SMTPTranscipient:        smtpTranscipient,
		SMTPRecipient:           smtpRecipient,
		SMTPNotificationSubject: smtpNotificationSubject,
	}, nil
}

func main() {
	parameters, err := getJobParameters()
	if err != nil {
		fmt.Printf("Error getting parameters: %s\n", err)
		return
	}

	job := NewSazenTeaCheckerJob(*parameters)
	if err := job.runJobLoop(); err != nil {
		log.Fatalf("%s", err)
	}
}
