//! Domain ownership proof: a TXT record `_litepod-verify.<domain>` holding the
//! token Litepod issued. Checked on demand, never cached.

use hickory_resolver::TokioResolver;
use hickory_resolver::proto::rr::RData;

/// The record's label relative to the zone: what DNS panels (Squarespace,
/// GoDaddy, Cloudflare…) expect in their "Host" field.
pub const RECORD_HOST: &str = "_litepod-verify";

pub fn record_name(domain: &str) -> String {
    format!("{RECORD_HOST}.{domain}")
}

/// Where the token may be found. Panels append the zone to the "Host" field,
/// so pasting the full name publishes `_litepod-verify.<domain>.<domain>`;
/// that name is still inside the zone, so it proves ownership just as well.
pub fn candidate_names(domain: &str) -> [String; 2] {
    let name = record_name(domain);
    let doubled = format!("{name}.{domain}");
    [name, doubled]
}

/// Every TXT string published at `name`. A lookup failure (NXDOMAIN, timeout)
/// is an empty list: the caller only asks "is the token there".
pub async fn txt_values(name: &str) -> Vec<String> {
    let Ok(builder) = TokioResolver::builder_tokio() else {
        return Vec::new();
    };
    let Ok(resolver) = builder.build() else {
        return Vec::new();
    };
    match resolver.txt_lookup(name).await {
        Ok(lookup) => lookup
            .answers()
            .iter()
            .filter_map(|record| match &record.data {
                RData::TXT(txt) => Some(txt),
                _ => None,
            })
            .map(|txt| {
                txt.txt_data
                    .iter()
                    .map(|part| String::from_utf8_lossy(part))
                    .collect::<String>()
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zone_appended_twice_is_also_checked() {
        assert_eq!(
            candidate_names("contoso.com"),
            [
                "_litepod-verify.contoso.com".to_string(),
                "_litepod-verify.contoso.com.contoso.com".to_string(),
            ]
        );
    }
}
