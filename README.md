# sazen-tea-checker

Helps me check for when Sazen has matcha!

Periodically scrapes a Sazen products page, follows each product link, and
emails me when a matching matcha product (by brand and ingredients) is listed.

## Notifications

An email only goes out when both are true:

1. At least one matching matcha product is listed.
2. The set of matching item codes differs from the last email that was sent.

Products new since the last email are tagged `(NEW!)` in the message. A product
dropping off the listing also counts as a change, so that email lists what is
currently available with nothing tagged new. The listing going empty sends no
email and resets the comparison, so a product returning later notifies again.

The comparison is held in memory only: restarting the job means the next
iteration that finds any products will email, even if nothing changed.

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
