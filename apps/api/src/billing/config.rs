#[derive(Clone, Debug)]
pub struct PolarConfig {
    pub environment: PolarEnvironment,
    pub access_token: String,
    pub webhook_secret: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolarEnvironment {
    Sandbox,
    Production,
}

impl PolarEnvironment {
    pub fn base_url(self) -> &'static str {
        match self {
            Self::Sandbox => "https://sandbox-api.polar.sh",
            Self::Production => "https://api.polar.sh",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Production => "production",
        }
    }
}

impl PolarConfig {
    pub fn from_env() -> Result<Option<Self>, String> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, String> {
        let read = |key: &str| {
            lookup(key)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let environment = match read("POLAR_ENVIRONMENT").as_deref() {
            None | Some("sandbox") => PolarEnvironment::Sandbox,
            Some("production" | "prod" | "live") => PolarEnvironment::Production,
            Some(_) => return Err("POLAR_ENVIRONMENT must be sandbox or production".into()),
        };

        // Local `.env` templates leave credentials empty. Treat that as billing
        // disabled rather than making every development boot fail.
        match (read("POLAR_ACCESS_TOKEN"), read("POLAR_WEBHOOK_SECRET")) {
            (None, None) => Ok(None),
            (Some(access_token), Some(webhook_secret)) => Ok(Some(Self {
                environment,
                access_token,
                webhook_secret,
            })),
            (None, _) => Err("POLAR_ACCESS_TOKEN is required when Polar is configured".into()),
            (_, None) => Err("POLAR_WEBHOOK_SECRET is required when Polar is configured".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PolarConfig, PolarEnvironment};
    use std::collections::HashMap;

    fn config(vars: &[(&str, &str)]) -> Result<Option<PolarConfig>, String> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        PolarConfig::from_lookup(|key| vars.get(key).cloned())
    }

    #[test]
    fn configured_environment_has_expected_base_url() {
        assert_eq!(
            PolarEnvironment::Sandbox.base_url(),
            "https://sandbox-api.polar.sh"
        );
        assert_eq!(
            PolarEnvironment::Production.base_url(),
            "https://api.polar.sh"
        );
    }

    #[test]
    fn environment_aliases_select_base_url() {
        let creds = [
            ("POLAR_ACCESS_TOKEN", "token"),
            ("POLAR_WEBHOOK_SECRET", "whsec_x"),
        ];
        for (value, expected) in [
            ("sandbox", PolarEnvironment::Sandbox),
            ("production", PolarEnvironment::Production),
            ("prod", PolarEnvironment::Production),
            ("live", PolarEnvironment::Production),
        ] {
            let mut vars = creds.to_vec();
            vars.push(("POLAR_ENVIRONMENT", value));
            assert_eq!(config(&vars).unwrap().unwrap().environment, expected);
        }
    }

    #[test]
    fn empty_pair_disables_and_half_pair_errors() {
        assert!(
            config(&[("POLAR_ENVIRONMENT", "sandbox")])
                .unwrap()
                .is_none()
        );
        assert!(config(&[("POLAR_ENVIRONMENT", "staging")]).is_err());
        assert!(config(&[("POLAR_ACCESS_TOKEN", "token")]).is_err());
    }
}
