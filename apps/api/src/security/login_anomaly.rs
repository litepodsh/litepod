//! Geolocates auth events (cloud only) and flags the ones that look wrong:
//!
//! - `new_country`: a sign-in from a country this account never signed in from.
//! - `impossible_travel`: two sign-ins farther apart than a plane could fly in between.
//! - `distributed_failures`: one email failing from many IPs in a short window
//!   (credential stuffing that the per-IP login ban does not see).
//!
//! A flag marks the row and alerts the root organization's notification channels.
//! Nothing is blocked automatically: VPNs and mobile carriers produce false
//! positives, so the SysAdmin decides.

use sqlx::Row;

use crate::state::state::SharedState;

/// Sign-ins: what the location history of an account is made of.
const SIGN_INS: [&str; 3] = ["auth.login", "auth.login_sso", "auth.registered"];
const FAILED: &str = "auth.login_failed";

/// Faster than an airliner, counting the time to get to the airport.
const MAX_SPEED_KMH: f64 = 900.0;
/// Below this, two IPs of the same region or carrier routinely look apart.
const MIN_TRAVEL_KM: f64 = 500.0;
/// Distinct IPs failing for one email inside the window before it is an attack.
const FAILURE_IPS: i64 = 5;
const FAILURE_WINDOW: &str = "-15 minutes";
/// How long an IP's resolved location is reused instead of asking ip.guide again.
const GEO_CACHE: &str = "-30 days";

pub fn is_located(action: &str) -> bool {
    action == FAILED || SIGN_INS.contains(&action)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    NewCountry,
    ImpossibleTravel,
    DistributedFailures,
}

impl Flag {
    pub fn key(self) -> &'static str {
        match self {
            Flag::NewCountry => "new_country",
            Flag::ImpossibleTravel => "impossible_travel",
            Flag::DistributedFailures => "distributed_failures",
        }
    }
}

struct Location {
    country: Option<String>,
    city: Option<String>,
    latitude: f64,
    longitude: f64,
    asn: Option<String>,
}

/// Great-circle distance in km.
fn distance_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (lat1, lon1) = (a.0.to_radians(), a.1.to_radians());
    let (lat2, lon2) = (b.0.to_radians(), b.1.to_radians());
    let h = ((lat2 - lat1) / 2.0).sin().powi(2)
        + lat1.cos() * lat2.cos() * ((lon2 - lon1) / 2.0).sin().powi(2);
    2.0 * 6371.0 * h.sqrt().asin()
}

fn is_impossible_travel(previous: (f64, f64), current: (f64, f64), hours_between: f64) -> bool {
    let km = distance_km(previous, current);
    // A minimum of a few minutes keeps two near-simultaneous logins from dividing by zero.
    km > MIN_TRAVEL_KM && km / hours_between.max(0.05) > MAX_SPEED_KMH
}

pub async fn locate_and_check(state: SharedState, event_id: String, ip: String) {
    if let Err(error) = run(&state, &event_id, &ip).await {
        tracing::warn!(%error, event_id, "security event geolocation failed");
    }
}

async fn run(state: &SharedState, event_id: &str, ip: &str) -> Result<(), sqlx::Error> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT action, user_id, metadata FROM security_events WHERE id = ?1")
        .bind(event_id)
        .fetch_optional(&conn)
        .await?;
    let Some(row) = row else { return Ok(()) };
    let action: String = row.try_get(0)?;
    let user_id: Option<String> = row.try_get(1)?;
    let metadata: Option<String> = row.try_get(2)?;

    let location = cached_location(state, event_id, ip).await?;
    let location = match location {
        Some(location) => Some(location),
        None => crate::servers::geoip::for_host(ip)
            .await
            .ok()
            .map(|geo| Location {
                country: geo.country,
                city: geo.city,
                latitude: geo.latitude,
                longitude: geo.longitude,
                asn: geo.asn,
            }),
    };
    if let Some(location) = &location {
        sqlx::query("UPDATE security_events SET country = ?1, city = ?2, latitude = ?3, longitude = ?4, asn = ?5 WHERE id = ?6")
            .bind(&location.country)
            .bind(&location.city)
            .bind(location.latitude)
            .bind(location.longitude)
            .bind(&location.asn)
            .bind(event_id)
            .execute(&conn)
            .await?;
    }

    let flag = if action == FAILED {
        match metadata.as_deref() {
            Some(email) => distributed_failures(state, email).await?,
            None => None,
        }
    } else {
        match (user_id.as_deref(), &location) {
            (Some(user_id), Some(location)) => {
                sign_in_flag(state, event_id, user_id, location).await?
            }
            _ => None,
        }
    };
    let Some(flag) = flag else { return Ok(()) };
    sqlx::query("UPDATE security_events SET flag = ?1 WHERE id = ?2")
        .bind(flag.key())
        .bind(event_id)
        .execute(&conn)
        .await?;

    let subject = match user_id.as_deref() {
        Some(user_id) => {
            sqlx::query_scalar::<_, String>("SELECT email FROM users WHERE id = ?1")
                .bind(user_id)
                .fetch_optional(&conn)
                .await?
        }
        None => metadata,
    };
    let place = location.as_ref().map(|location| {
        [location.city.as_deref(), location.country.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ")
    });
    crate::notifications::emit::notify_security_alert(
        state,
        flag,
        subject.as_deref().unwrap_or("unknown"),
        ip,
        place.as_deref(),
    )
    .await;
    Ok(())
}

/// The last location seen for this IP, so a busy office IP is looked up once a month.
async fn cached_location(
    state: &SharedState,
    event_id: &str,
    ip: &str,
) -> Result<Option<Location>, sqlx::Error> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT country, city, latitude, longitude, asn FROM security_events
         WHERE source_ip = ?1 AND id != ?2 AND latitude IS NOT NULL
           AND created_at > datetime('now', '{GEO_CACHE}')
         ORDER BY id DESC LIMIT 1"
    ))
    .bind(ip)
    .bind(event_id)
    .fetch_optional(&conn)
    .await?;
    row.map(|row| {
        Ok(Location {
            country: row.try_get(0)?,
            city: row.try_get(1)?,
            latitude: row.try_get(2)?,
            longitude: row.try_get(3)?,
            asn: row.try_get(4)?,
        })
    })
    .transpose()
}

async fn sign_in_flag(
    state: &SharedState,
    event_id: &str,
    user_id: &str,
    location: &Location,
) -> Result<Option<Flag>, sqlx::Error> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let sign_ins = SIGN_INS.map(|action| format!("'{action}'")).join(", ");
    // UUIDv7 ids sort by time, so `id < ?2` is "before this event".
    let previous = sqlx::query(&format!(
        "SELECT latitude, longitude, (julianday('now') - julianday(created_at)) * 24.0
         FROM security_events
         WHERE user_id = ?1 AND id < ?2 AND action IN ({sign_ins}) AND latitude IS NOT NULL
         ORDER BY id DESC LIMIT 1"
    ))
    .bind(user_id)
    .bind(event_id)
    .fetch_optional(&conn)
    .await?;
    // First located sign-in: it becomes the baseline, nothing to compare against.
    let Some(previous) = previous else {
        return Ok(None);
    };
    let from = (
        previous.try_get::<f64, _>(0)?,
        previous.try_get::<f64, _>(1)?,
    );
    let hours: f64 = previous.try_get(2)?;
    if is_impossible_travel(from, (location.latitude, location.longitude), hours) {
        return Ok(Some(Flag::ImpossibleTravel));
    }
    let Some(country) = location.country.as_deref() else {
        return Ok(None);
    };
    let seen: bool = sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM security_events
         WHERE user_id = ?1 AND id < ?2 AND action IN ({sign_ins}) AND country = ?3)"
    ))
    .bind(user_id)
    .bind(event_id)
    .bind(country)
    .fetch_one(&conn)
    .await?;
    Ok((!seen).then_some(Flag::NewCountry))
}

async fn distributed_failures(
    state: &SharedState,
    email: &str,
) -> Result<Option<Flag>, sqlx::Error> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let ips: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(DISTINCT source_ip) FROM security_events
         WHERE action = '{FAILED}' AND metadata = ?1 AND created_at > datetime('now', '{FAILURE_WINDOW}')"
    ))
    .bind(email)
    .fetch_one(&conn)
    .await?;
    // Exactly at the threshold: one alert per attack, not one per extra attempt.
    Ok((ips == FAILURE_IPS).then_some(Flag::DistributedFailures))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEXICO_CITY: (f64, f64) = (19.4326, -99.1332);
    const GUADALAJARA: (f64, f64) = (20.6597, -103.3496);
    const MADRID: (f64, f64) = (40.4168, -3.7038);

    #[test]
    fn distance_is_roughly_right() {
        let km = distance_km(MEXICO_CITY, MADRID);
        assert!((9000.0..9100.0).contains(&km), "{km}");
    }

    #[test]
    fn flags_a_continent_hop_in_an_hour() {
        assert!(is_impossible_travel(MEXICO_CITY, MADRID, 1.0));
    }

    #[test]
    fn allows_a_real_flight() {
        assert!(!is_impossible_travel(MEXICO_CITY, MADRID, 14.0));
    }

    #[test]
    fn ignores_nearby_cities_even_at_once() {
        // ~460 km: inside the noise of carrier and VPN exits.
        assert!(!is_impossible_travel(MEXICO_CITY, GUADALAJARA, 0.0));
    }

    async fn insert(
        state: &SharedState,
        id: &str,
        action: &str,
        ago: &str,
        at: Option<(f64, f64, &str)>,
    ) {
        let conn = crate::git::repositories::db::get_conn(state).await;
        sqlx::query(&format!(
            "INSERT INTO security_events (id, user_id, action, metadata, source_ip, latitude, longitude, country, created_at)
             VALUES (?1, 'u', ?2, 'u@x.test', ?1, ?3, ?4, ?5, datetime('now', '{ago}'))"
        ))
        .bind(id)
        .bind(action)
        .bind(at.map(|at| at.0))
        .bind(at.map(|at| at.1))
        .bind(at.map(|at| at.2))
        .execute(&conn)
        .await
        .unwrap();
    }

    fn here(at: (f64, f64), country: &str) -> Location {
        Location {
            country: Some(country.into()),
            city: None,
            latitude: at.0,
            longitude: at.1,
            asn: None,
        }
    }

    #[tokio::test]
    async fn sign_in_flags_against_history() {
        let (state, path) = crate::testing::test_state().await;
        let conn = crate::git::repositories::db::get_conn(&state).await;
        sqlx::query("INSERT INTO users (id, email, hashed_password, name, status_id) VALUES ('u', 'u@x.test', 'h', 'U', 1)")
            .execute(&conn).await.unwrap();

        // First located sign-in is only a baseline.
        assert_eq!(
            sign_in_flag(&state, "0001", "u", &here(MEXICO_CITY, "Mexico"))
                .await
                .unwrap(),
            None
        );
        insert(
            &state,
            "0001",
            "auth.login",
            "-2 days",
            Some((MEXICO_CITY.0, MEXICO_CITY.1, "Mexico")),
        )
        .await;

        assert_eq!(
            sign_in_flag(&state, "0002", "u", &here(GUADALAJARA, "Mexico"))
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            sign_in_flag(&state, "0002", "u", &here(MADRID, "Spain"))
                .await
                .unwrap(),
            Some(Flag::NewCountry),
            "two days is enough to fly, but Spain is new"
        );
        insert(
            &state,
            "0002",
            "auth.login",
            "-1 hours",
            Some((MADRID.0, MADRID.1, "Spain")),
        )
        .await;
        assert_eq!(
            sign_in_flag(&state, "0003", "u", &here(MEXICO_CITY, "Mexico"))
                .await
                .unwrap(),
            Some(Flag::ImpossibleTravel)
        );
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn alerts_once_when_failures_cross_the_threshold() {
        let (state, path) = crate::testing::test_state().await;
        let conn = crate::git::repositories::db::get_conn(&state).await;
        sqlx::query("INSERT INTO users (id, email, hashed_password, name, status_id) VALUES ('u', 'u@x.test', 'h', 'U', 1)")
            .execute(&conn).await.unwrap();
        for n in 1..=FAILURE_IPS + 1 {
            insert(&state, &format!("f{n}"), FAILED, "-1 minutes", None).await;
            let flag = distributed_failures(&state, "u@x.test").await.unwrap();
            assert_eq!(flag.is_some(), n == FAILURE_IPS, "attempt {n}");
        }
        crate::testing::cleanup_database(&path);
    }

    #[test]
    fn locates_sign_ins_and_failures_only() {
        assert!(is_located("auth.login"));
        assert!(is_located("auth.login_sso"));
        assert!(is_located("auth.login_failed"));
        assert!(!is_located("auth.logout"));
        assert!(!is_located("password.changed"));
    }
}
