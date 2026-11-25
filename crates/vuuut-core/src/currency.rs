//! Site currency and monetary units used by store checkout and Stripe.
pub const SETTING: &str = "site_currency";
pub const DEFAULT: &str = "USD";

/// Supported site currencies and their names.
pub const OPTIONS: &[(&str, &str)] = &[
    ("AED", "AED · UAE dirham"),
    ("ARS", "ARS · Argentine peso"),
    ("AUD", "AUD · Australian dollar"),
    ("BDT", "BDT · Bangladeshi taka"),
    ("BRL", "BRL · Brazilian real"),
    ("CAD", "CAD · Canadian dollar"),
    ("CHF", "CHF · Swiss franc"),
    ("CLP", "CLP · Chilean peso"),
    ("CNY", "CNY · Chinese yuan"),
    ("COP", "COP · Colombian peso"),
    ("CZK", "CZK · Czech koruna"),
    ("DKK", "DKK · Danish krone"),
    ("EGP", "EGP · Egyptian pound"),
    ("EUR", "EUR · Euro"),
    ("GBP", "GBP · British pound"),
    ("HKD", "HKD · Hong Kong dollar"),
    ("HUF", "HUF · Hungarian forint"),
    ("IDR", "IDR · Indonesian rupiah"),
    ("ILS", "ILS · Israeli new shekel"),
    ("INR", "INR · Indian rupee"),
    ("JPY", "JPY · Japanese yen"),
    ("KES", "KES · Kenyan shilling"),
    ("KRW", "KRW · South Korean won"),
    ("LKR", "LKR · Sri Lankan rupee"),
    ("MAD", "MAD · Moroccan dirham"),
    ("MXN", "MXN · Mexican peso"),
    ("MYR", "MYR · Malaysian ringgit"),
    ("NGN", "NGN · Nigerian naira"),
    ("NOK", "NOK · Norwegian krone"),
    ("NPR", "NPR · Nepalese rupee"),
    ("NZD", "NZD · New Zealand dollar"),
    ("PEN", "PEN · Peruvian sol"),
    ("PHP", "PHP · Philippine peso"),
    ("PKR", "PKR · Pakistani rupee"),
    ("PLN", "PLN · Polish złoty"),
    ("QAR", "QAR · Qatari riyal"),
    ("RON", "RON · Romanian leu"),
    ("SAR", "SAR · Saudi riyal"),
    ("SEK", "SEK · Swedish krona"),
    ("SGD", "SGD · Singapore dollar"),
    ("THB", "THB · Thai baht"),
    ("TRY", "TRY · Turkish lira"),
    ("TWD", "TWD · New Taiwan dollar"),
    ("UAH", "UAH · Ukrainian hryvnia"),
    ("USD", "USD · US dollar"),
    ("UYU", "UYU · Uruguayan peso"),
    ("VND", "VND · Vietnamese đồng"),
    ("XAF", "XAF · Central African CFA franc"),
    ("XOF", "XOF · West African CFA franc"),
    ("ZAR", "ZAR · South African rand"),
];

/// Number of minor units Stripe expects per whole unit for these currencies.
pub fn minor_unit_scale(currency: &str) -> i64 {
    match currency {
        "CLP" | "JPY" | "KRW" | "VND" | "XAF" | "XOF" => 1,
        _ => 100,
    }
}

pub fn normalize(value: &str) -> Option<String> {
    let code = value.trim().trim_matches('"').to_ascii_uppercase();
    OPTIONS
        .iter()
        .any(|(option, _)| *option == code)
        .then_some(code)
}

pub fn configured(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    get(SETTING).and_then(|code| normalize(&code))
}

pub fn preferred(get: impl Fn(&str) -> Option<String>) -> String {
    configured(&get)
        .or_else(|| get("ecom_currency").and_then(|code| normalize(&code)))
        .unwrap_or_else(|| DEFAULT.into())
}

pub fn donations(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    configured(&get).or_else(|| get("donations_currency").filter(|code| !code.is_empty()))
}

/// Directory and course editors store hundredths even for whole-unit currencies.
pub fn fixed_price_to_minor(amount: i64, currency: &str) -> Option<i64> {
    let divisor = 100 / minor_unit_scale(currency);
    (amount % divisor == 0).then_some(amount / divisor)
}
