# sazen-tea-checker

Helps me check for when Sazen has matcha!

Periodically scrapes a Sazen products page, follows each product link, and
emails me when a matching matcha product (by brand and ingredients) is listed.

## Build & run

```sh
go build -o sazen-tea-checker .
./sazen-tea-checker
```

## Configuration

All configuration is read from environment variables:

| Variable | Description |
| --- | --- |
| `JOB_INTERVAL_MINUTES` | Minutes to wait between check iterations (integer) |
| `PRODUCTS_URL` | URL of the products listing page to scrape |
| `MATCHA_BRANDS` | Comma-separated brand keywords to match (lowercase) |
| `SMTP_URL` | SMTP server host (e.g. `smtp.gmail.com`) |
| `SMTP_USER` | SMTP username |
| `SMTP_PASSWORD` | SMTP password |
| `SMTP_TRANSCIPIENT` | From address |
| `SMTP_RECIPIENT` | To address |
| `SMTP_NOTIFICATION_SUBJECT` | Subject line for the notification email |
