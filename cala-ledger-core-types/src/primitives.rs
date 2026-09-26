use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use rusty_money::{crypto, iso, Locale};
use serde::{Deserialize, Serialize};

use cel_interpreter::{CelResult, CelType, CelValue, ResultCoercionError};

es_entity::entity_id! { AccountId }
impl From<AccountId> for cel_interpreter::CelValue {
    fn from(id: AccountId) -> Self {
        cel_interpreter::CelValue::Uuid(id.0)
    }
}
es_entity::entity_id! { AccountSetId }
impl From<AccountSetId> for cel_interpreter::CelValue {
    fn from(id: AccountSetId) -> Self {
        cel_interpreter::CelValue::Uuid(id.0)
    }
}
es_entity::entity_id! { JournalId }
impl From<JournalId> for cel_interpreter::CelValue {
    fn from(id: JournalId) -> Self {
        cel_interpreter::CelValue::Uuid(id.0)
    }
}
es_entity::entity_id! { TxTemplateId }
impl From<TxTemplateId> for cel_interpreter::CelValue {
    fn from(id: TxTemplateId) -> Self {
        cel_interpreter::CelValue::Uuid(id.0)
    }
}
es_entity::entity_id! { TransactionId }
impl From<TransactionId> for cel_interpreter::CelValue {
    fn from(id: TransactionId) -> Self {
        cel_interpreter::CelValue::Uuid(id.0)
    }
}
es_entity::entity_id! { EntryId }
impl From<EntryId> for cel_interpreter::CelValue {
    fn from(id: EntryId) -> Self {
        cel_interpreter::CelValue::Uuid(id.0)
    }
}
es_entity::entity_id! { VelocityLimitId }
es_entity::entity_id! { VelocityControlId }

pub type BalanceId = (JournalId, AccountId, Currency);
impl From<&AccountSetId> for AccountId {
    fn from(id: &AccountSetId) -> Self {
        Self(id.0)
    }
}
impl From<AccountSetId> for AccountId {
    fn from(id: AccountSetId) -> Self {
        Self(id.0)
    }
}

#[derive(
    Default,
    Debug,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    PartialEq,
    Eq,
    sqlx::Type,
    strum::Display,
    strum::EnumString,
)]
#[sqlx(type_name = "DebitOrCredit", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum DebitOrCredit {
    Debit,
    #[default]
    Credit,
}

impl TryFrom<CelResult<'_>> for DebitOrCredit {
    type Error = ResultCoercionError;

    fn try_from(CelResult { expr, val }: CelResult) -> Result<Self, Self::Error> {
        match val {
            CelValue::String(v) if v.as_ref() == "DEBIT" => Ok(DebitOrCredit::Debit),
            CelValue::String(v) if v.as_ref() == "CREDIT" => Ok(DebitOrCredit::Credit),
            v => Err(ResultCoercionError::BadExternalTypeCoercion(
                format!("{expr:?}"),
                CelType::from(&v),
                "DebitOrCredit",
            )),
        }
    }
}

impl From<DebitOrCredit> for CelValue {
    fn from(v: DebitOrCredit) -> Self {
        match v {
            DebitOrCredit::Debit => "DEBIT".into(),
            DebitOrCredit::Credit => "CREDIT".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BalanceRollup {
    /// Rolled up inside every posting to a member account, under an
    /// exclusive lock per (journal, set, currency).
    Synchronous,
    /// Skipped at posting time; refreshed by recalculating the sets
    /// returned from `list_eventually_consistent_ids`.
    EventuallyConsistent,
}

#[derive(Default, Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "Status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum Status {
    #[default]
    Active,
    Locked,
}

#[derive(Default, Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, sqlx::Type)]
#[sqlx(type_name = "Layer", rename_all = "snake_case")]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum Layer {
    #[default]
    Settled,
    Pending,
    Encumbrance,
}

#[derive(thiserror::Error, Debug)]
pub enum ParseLayerError {
    #[error("CalaCoreTypeError - UnknownLayer: {0:?}")]
    UnknownLayer(String),
}

impl TryFrom<CelResult<'_>> for Layer {
    type Error = ResultCoercionError;

    fn try_from(CelResult { expr, val }: CelResult) -> Result<Self, Self::Error> {
        match val {
            CelValue::String(v) if v.as_ref() == "SETTLED" => Ok(Layer::Settled),
            CelValue::String(v) if v.as_ref() == "PENDING" => Ok(Layer::Pending),
            CelValue::String(v) if v.as_ref() == "ENCUMBRANCE" => Ok(Layer::Encumbrance),
            v => Err(ResultCoercionError::BadExternalTypeCoercion(
                format!("{expr:?}"),
                CelType::from(&v),
                "Layer",
            )),
        }
    }
}

impl From<Layer> for CelValue {
    fn from(l: Layer) -> Self {
        match l {
            Layer::Settled => "SETTLED".into(),
            Layer::Pending => "PENDING".into(),
            Layer::Encumbrance => "ENCUMBRANCE".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Serialize, Deserialize)]
#[serde(try_from = "String")]
#[serde(into = "&str")]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum Currency {
    Iso(&'static iso::Currency),
    Crypto(&'static crypto::Currency),
}

impl Currency {
    pub const BTC: Self = Self::Crypto(crypto::BTC);
    pub const USD: Self = Self::Iso(iso::USD);

    pub fn code(&self) -> &'static str {
        match self {
            Currency::Iso(c) => c.iso_alpha_code,
            Currency::Crypto(c) => c.code,
        }
    }

    /// Register a consumer currency for `FromStr` / serde / CEL.
    ///
    /// Leaks the code once into process memory so `Currency` can stay `Copy`.
    /// Built-in ISO and crypto table codes cannot be shadowed.
    pub fn register(code: &str) -> Result<Self, ParseCurrencyError> {
        let code = code.trim();
        if code.is_empty() {
            return Err(ParseCurrencyError::UnknownCurrency(code.to_string()));
        }
        if iso::find(code).is_some() || crypto::find(code).is_some() {
            return Err(ParseCurrencyError::UnknownCurrency(code.to_string()));
        }

        let mut registry = custom_currency_registry()
            .lock()
            .expect("currency registry poisoned");
        if let Some(existing) = registry.get(code) {
            return Ok(*existing);
        }

        let leaked_code: &'static str = Box::leak(code.to_owned().into_boxed_str());
        let leaked_currency: &'static crypto::Currency = Box::leak(Box::new(crypto::Currency {
            code: leaked_code,
            exponent: 0,
            locale: Locale::EnUs,
            minor_units: 1,
            name: leaked_code,
            symbol: leaked_code,
            symbol_first: false,
        }));
        let registered = Self::Crypto(leaked_currency);
        registry.insert(leaked_code, registered);
        Ok(registered)
    }

    fn find_custom(code: &str) -> Option<Self> {
        custom_currency_registry()
            .lock()
            .expect("currency registry poisoned")
            .get(code)
            .copied()
    }
}

fn custom_currency_registry() -> &'static Mutex<HashMap<&'static str, Currency>> {
    static REGISTRY: OnceLock<Mutex<HashMap<&'static str, Currency>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

impl std::fmt::Display for Currency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.code())
    }
}

impl From<Currency> for CelValue {
    fn from(c: Currency) -> Self {
        c.code().into()
    }
}

impl std::hash::Hash for Currency {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.code().hash(state);
    }
}

impl PartialEq for Currency {
    fn eq(&self, other: &Self) -> bool {
        self.code() == other.code()
    }
}

impl Ord for Currency {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.code().cmp(other.code())
    }
}

impl PartialOrd for Currency {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ParseCurrencyError {
    #[error("CalaCoreTypeError - UnknownCurrency: {0}")]
    UnknownCurrency(String),
}

impl std::str::FromStr for Currency {
    type Err = ParseCurrencyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(c) = iso::find(s) {
            return Ok(Currency::Iso(c));
        }
        if let Some(c) = crypto::find(s) {
            return Ok(Currency::Crypto(c));
        }
        if let Some(c) = Self::find_custom(s) {
            return Ok(c);
        }
        Err(ParseCurrencyError::UnknownCurrency(s.to_string()))
    }
}

impl TryFrom<String> for Currency {
    type Error = ParseCurrencyError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<Currency> for &'static str {
    fn from(c: Currency) -> Self {
        c.code()
    }
}

impl TryFrom<CelResult<'_>> for Currency {
    type Error = ResultCoercionError;

    fn try_from(CelResult { expr, val }: CelResult) -> Result<Self, Self::Error> {
        match val {
            CelValue::String(v) => v.as_ref().parse::<Currency>().map_err(|e| {
                ResultCoercionError::ExternalTypeCoercionError(
                    format!("{expr:?}"),
                    format!("{v:?}"),
                    "Currency",
                    format!("{e:?}"),
                )
            }),
            v => Err(ResultCoercionError::BadExternalTypeCoercion(
                format!("{expr:?}"),
                CelType::from(&v),
                "Currency",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::primitives::{Currency, ParseCurrencyError};

    #[test]
    fn currency_constants() {
        assert_eq!(Currency::USD, "USD".parse().unwrap());
        assert_eq!(Currency::BTC, "BTC".parse().unwrap());
    }

    #[test]
    fn register_custom_currency_parses() {
        let registered = Currency::register("FOO").unwrap();
        assert_eq!(registered.code(), "FOO");
        assert_eq!("FOO".parse::<Currency>().unwrap(), registered);
        assert_eq!(registered, Currency::register("FOO").unwrap());
    }

    #[test]
    fn register_custom_currency_serde_round_trip() {
        let registered = Currency::register("XYZ").unwrap();
        let json = serde_json::to_string(&registered).unwrap();
        assert_eq!(json, "\"XYZ\"");
        let back: Currency = serde_json::from_str(&json).unwrap();
        assert_eq!(back, registered);
        assert_eq!(back.code(), "XYZ");
    }

    #[test]
    fn register_rejects_iso_and_crypto_shadow() {
        assert!(matches!(
            Currency::register("USD"),
            Err(ParseCurrencyError::UnknownCurrency(_))
        ));
        assert!(matches!(
            Currency::register("BTC"),
            Err(ParseCurrencyError::UnknownCurrency(_))
        ));
        assert!(matches!(
            "USD".parse::<Currency>().unwrap(),
            Currency::Iso(_)
        ));
        assert!(matches!(
            "BTC".parse::<Currency>().unwrap(),
            Currency::Crypto(_)
        ));
    }

    #[test]
    fn register_rejects_empty_or_blank() {
        assert!(Currency::register("").is_err());
        assert!(Currency::register("   ").is_err());
    }

    #[test]
    fn unregistered_currency_still_unknown() {
        assert!(matches!(
            "NOTAREALCURRENCYCODE".parse::<Currency>(),
            Err(ParseCurrencyError::UnknownCurrency(_))
        ));
    }
}
