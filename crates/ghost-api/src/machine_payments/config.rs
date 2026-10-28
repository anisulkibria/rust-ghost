use super::*;
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub amount: i64,
    pub currency: String,
    pub stripe_profile_id: String,
    pub tempo_enabled: bool,
    pub tempo_testnet: bool,
    pub x402_enabled: bool,
    pub x402_network: String,
    pub x402_facilitator: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            amount: 100,
            currency: "USD".into(),
            stripe_profile_id: String::new(),
            tempo_enabled: false,
            tempo_testnet: false,
            x402_enabled: false,
            x402_network: "eip155:8453".into(),
            x402_facilitator: "https://facilitator.xpay.sh".into(),
        }
    }
}
impl Config {
    pub fn load(state: &AppState) -> Self {
        let s = |key: &str| state.settings.get_str(key).unwrap_or_default();
        Self {
            enabled: state.settings.get_bool("machine_payments_enabled"),
            amount: s("machine_payments_amount").parse().unwrap_or(100),
            currency: match s("machine_payments_currency") {
                v if !v.is_empty() => v,
                _ => "USD".into(),
            },
            stripe_profile_id: s("machine_payments_stripe_profile_id"),
            tempo_enabled: state.settings.get_bool("machine_payments_tempo_enabled"),
            tempo_testnet: state.settings.get_bool("machine_payments_tempo_testnet"),
            x402_enabled: state.settings.get_bool("machine_payments_x402_enabled"),
            x402_network: match s("machine_payments_x402_network") {
                v if !v.is_empty() => v,
                _ => "eip155:8453".into(),
            },
            x402_facilitator: match s("machine_payments_x402_facilitator") {
                v if !v.is_empty() => v,
                _ => "https://facilitator.xpay.sh".into(),
            },
        }
    }
    pub fn validate(&self) -> Result<()> {
        if !(1..=1_000_000).contains(&self.amount)
            || self.currency.len() != 3
            || !self.currency.bytes().all(|c| c.is_ascii_alphabetic())
            || self.stripe_profile_id.len() > 191
        {
            return Err(bad(
                "Enter an amount from 1 to 1000000 minor units and a three-letter currency",
            ));
        }
        if !matches!(self.x402_network.as_str(), "eip155:8453" | "eip155:84532") {
            return Err(bad("Choose Base mainnet or Base Sepolia"));
        }
        let facilitator = reqwest::Url::parse(&self.x402_facilitator)
            .map_err(|_| bad("Invalid facilitator URL"))?;
        if facilitator.scheme() != "https"
            || facilitator.host_str().is_none()
            || !facilitator.username().is_empty()
            || facilitator.password().is_some()
            || facilitator.query().is_some()
            || facilitator.fragment().is_some()
        {
            return Err(bad("The settlement facilitator must use HTTPS"));
        }
        if self.x402_network == "eip155:8453" && facilitator.host_str() == Some("x402.org") {
            return Err(bad(
                "The x402.org facilitator is for testnet, not Base mainnet",
            ));
        }
        Ok(())
    }
    pub fn amount_string(&self) -> String {
        format!("{}.{:02}", self.amount / 100, self.amount % 100)
    }
}
pub fn address(state: &AppState, network: &str) -> Option<String> {
    let raw = state
        .settings
        .get("machine_payments_deposit_address")
        .unwrap_or(json!({}));
    let map = if let Some(raw) = raw.as_str() {
        serde_json::from_str::<Value>(raw).unwrap_or_else(|_| json!({"tempo":raw}))
    } else {
        raw
    };
    map[network]
        .as_str()
        .filter(|value| valid_address(value))
        .map(str::to_owned)
}
pub fn valid_address(value: &str) -> bool {
    value.len() == 42
        && value.starts_with("0x")
        && value[2..].bytes().all(|c| c.is_ascii_hexdigit())
        && value[2..].bytes().any(|c| c != b'0')
}
pub fn secret(state: &AppState) -> Option<String> {
    state
        .settings
        .get_str("machine_payments_secret")
        .filter(|secret| secret.len() >= 32)
}
pub fn realm(state: &AppState) -> Result<String> {
    let url = reqwest::Url::parse(&state.site_url()).map_err(|_| bad("Invalid site URL"))?;
    let host = url.host_str().ok_or_else(|| bad("Missing site host"))?;
    Ok(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    })
}
