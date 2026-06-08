use crate::config::SnmpConfig;
use async_snmp::{
    v3::{AuthProtocol, PrivProtocol},
    Auth, Client, Oid, Retry, Value,
};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SnmpError {
    #[error("invalid OID {oid}: {details}")]
    InvalidOid { oid: String, details: String },
    #[error("unsupported SNMP version: {0}")]
    UnsupportedVersion(String),
    #[error("unsupported SNMPv3 security level: {0}")]
    UnsupportedSecurityLevel(String),
    #[error("unsupported SNMPv3 auth protocol: {0}")]
    UnsupportedAuthProtocol(String),
    #[error("unsupported SNMPv3 privacy protocol: {0}")]
    UnsupportedPrivacyProtocol(String),
    #[error("missing {field} for SNMPv3 security level {security_level}")]
    MissingSecurityField {
        field: &'static str,
        security_level: String,
    },
    #[error("SNMP protocol error: {0}")]
    Protocol(#[from] Box<async_snmp::Error>),
    #[error("failed to create async runtime for SNMP client: {0}")]
    Runtime(std::io::Error),
    #[error("SNMP response returned {actual} values for {expected} requested OIDs")]
    WrongValueCount { expected: usize, actual: usize },
}

pub trait SnmpClient {
    fn get_many(&self, oids: &[String]) -> Result<Vec<String>, SnmpError>;
}

#[derive(Debug, Clone)]
pub struct RustSnmpClient {
    config: SnmpConfig,
}

impl RustSnmpClient {
    pub fn new(config: SnmpConfig) -> Self {
        Self { config }
    }

    fn build_auth(&self) -> Result<Auth, SnmpError> {
        match self.config.version.trim() {
            "1" => Ok(Auth::v1(&self.config.username)),
            "2" | "2c" => Ok(Auth::v2c(&self.config.username)),
            "3" => build_v3_auth(&self.config),
            other => Err(SnmpError::UnsupportedVersion(other.into())),
        }
    }
}

impl SnmpClient for RustSnmpClient {
    fn get_many(&self, oids: &[String]) -> Result<Vec<String>, SnmpError> {
        let parsed_oids = parse_oids(oids)?;
        let auth = self.build_auth()?;
        let config = self.config.clone();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(SnmpError::Runtime)?;

        rt.block_on(async move {
            let client = Client::builder((config.address.as_str(), config.port), auth)
                .timeout(config.timeout)
                .retry(Retry::fixed(u32::from(config.retries), Duration::ZERO))
                .connect()
                .await?;

            let varbinds = client.get_many(&parsed_oids).await?;
            let values: Vec<String> = varbinds
                .into_iter()
                .map(|varbind| value_to_string(varbind.value))
                .collect();

            if values.len() != oids.len() {
                return Err(SnmpError::WrongValueCount {
                    expected: oids.len(),
                    actual: values.len(),
                });
            }

            Ok(values)
        })
    }
}

pub fn build_v3_auth(config: &SnmpConfig) -> Result<Auth, SnmpError> {
    let level = config.security_level.trim().to_ascii_lowercase();
    match level.as_str() {
        "noauthnopriv" => Ok(Auth::usm(&config.username).into()),
        "authnopriv" => {
            let auth_password = required_secret(
                &config.auth_password,
                "auth_password",
                &config.security_level,
            )?;
            Ok(Auth::usm(&config.username)
                .auth(
                    parse_auth_protocol(config.auth_protocol.as_deref())?,
                    auth_password,
                )
                .into())
        }
        "authpriv" => {
            let auth_password = required_secret(
                &config.auth_password,
                "auth_password",
                &config.security_level,
            )?;
            let privacy_password = required_secret(
                &config.privacy_password,
                "privacy_password",
                &config.security_level,
            )?;
            Ok(Auth::usm(&config.username)
                .auth(
                    parse_auth_protocol(config.auth_protocol.as_deref())?,
                    auth_password,
                )
                .privacy(
                    parse_privacy_protocol(config.privacy_protocol.as_deref())?,
                    privacy_password,
                )
                .into())
        }
        other => Err(SnmpError::UnsupportedSecurityLevel(other.into())),
    }
}

fn required_secret<'a>(
    value: &'a Option<String>,
    field: &'static str,
    security_level: &str,
) -> Result<&'a String, SnmpError> {
    value
        .as_ref()
        .filter(|secret| !secret.is_empty())
        .ok_or_else(|| SnmpError::MissingSecurityField {
            field,
            security_level: security_level.into(),
        })
}

fn parse_auth_protocol(protocol: Option<&str>) -> Result<AuthProtocol, SnmpError> {
    match protocol
        .unwrap_or("sha1")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "md5" => Ok(AuthProtocol::Md5),
        "sha" | "sha1" => Ok(AuthProtocol::Sha1),
        "sha224" => Ok(AuthProtocol::Sha224),
        "sha256" => Ok(AuthProtocol::Sha256),
        "sha384" => Ok(AuthProtocol::Sha384),
        "sha512" => Ok(AuthProtocol::Sha512),
        other => Err(SnmpError::UnsupportedAuthProtocol(other.into())),
    }
}

fn parse_privacy_protocol(protocol: Option<&str>) -> Result<PrivProtocol, SnmpError> {
    match protocol
        .unwrap_or("aes128")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "des" => Ok(PrivProtocol::Des),
        "aes" | "aes128" => Ok(PrivProtocol::Aes128),
        "aes192" => Ok(PrivProtocol::Aes192),
        "aes256" => Ok(PrivProtocol::Aes256),
        other => Err(SnmpError::UnsupportedPrivacyProtocol(other.into())),
    }
}

fn parse_oids(oids: &[String]) -> Result<Vec<Oid>, SnmpError> {
    oids.iter()
        .map(|oid| {
            oid.parse::<Oid>().map_err(|source| SnmpError::InvalidOid {
                oid: oid.clone(),
                details: format!("{source:?}"),
            })
        })
        .collect()
}

fn value_to_string(value: Value) -> String {
    match value {
        Value::Integer(value) => value.to_string(),
        Value::OctetString(bytes) => match std::str::from_utf8(&bytes) {
            Ok(text) => text.to_string(),
            Err(_) => bytes_to_hex(&bytes),
        },
        Value::Null => "null".into(),
        Value::ObjectIdentifier(oid) => oid.to_string(),
        Value::IpAddress(value) => format!("{}.{}.{}.{}", value[0], value[1], value[2], value[3]),
        Value::Counter32(value) | Value::Gauge32(value) | Value::TimeTicks(value) => {
            value.to_string()
        }
        Value::Counter64(value) => value.to_string(),
        Value::Opaque(bytes) => bytes_to_hex(&bytes),
        Value::NoSuchObject => "no_such_object".into(),
        Value::NoSuchInstance => "no_such_instance".into(),
        Value::EndOfMibView => "end_of_mib_view".into(),
        Value::Unknown { tag, data } => {
            format!("unknown(tag=0x{tag:02x}, data={})", bytes_to_hex(&data))
        }
        other => format!("{other}"),
    }
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_oid_strings() {
        let oids = parse_oids(&["1.3.6.1.2.1.1.1.0".into()]).unwrap();

        assert_eq!(oids[0].to_string(), "1.3.6.1.2.1.1.1.0");
    }

    #[test]
    fn builds_no_auth_no_priv_security() {
        let config = SnmpConfig::default();

        build_v3_auth(&config).unwrap();
    }

    #[test]
    fn rejects_missing_auth_password_for_auth_no_priv() {
        let mut config = SnmpConfig {
            security_level: "authNoPriv".into(),
            ..SnmpConfig::default()
        };
        config.auth_password = None;

        assert!(build_v3_auth(&config).is_err());
    }
}
