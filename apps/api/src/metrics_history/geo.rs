//! IP geolocation for request analytics, from a MaxMind-format (`.mmdb`) city database.
//!
//! By default the API keeps DB-IP's free "IP to City Lite" database (CC BY 4.0, so the
//! UI credits it) next to the DuckDB file and refreshes it monthly. Umami reads the
//! same format (MaxMind GeoLite2), which needs a license key; either file works here.
//!
//! Env: `LITEPOD_GEOIP_DB` (path to your own `.mmdb`, never downloaded or replaced;
//! `off` disables geolocation).

use std::{
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, RwLock},
    time::{Duration, SystemTime},
};

use maxminddb::{Mmap, Reader, geoip2};

/// Where an IP is, as far as the database knows.
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct Geo {
    /// ISO 3166-1 alpha-2, e.g. `MX`.
    pub country: Option<String>,
    /// First-level subdivision (state, province), English name.
    pub region: Option<String>,
    pub city: Option<String>,
}

static READER: LazyLock<RwLock<Option<Arc<Reader<Mmap>>>>> = LazyLock::new(RwLock::default);

const DOWNLOAD_URL: &str = "https://download.db-ip.com/free/dbip-city-lite";
const FILE_NAME: &str = "dbip-city-lite.mmdb";
/// DB-IP publishes on the 1st of each month; refresh a little after that.
const MAX_AGE: Duration = Duration::from_secs(32 * 86_400);
const CHECK_EVERY: Duration = Duration::from_secs(86_400);

/// Empty for private addresses, unknown IPs, or when no database is loaded.
pub fn lookup(ip: &str) -> Geo {
    let Ok(address) = ip.parse::<IpAddr>() else {
        return Geo::default();
    };
    let reader = READER
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let Some(reader) = reader else {
        return Geo::default();
    };
    let Ok(Some(city)) = reader
        .lookup(address)
        .and_then(|result| result.decode::<geoip2::City>())
    else {
        return Geo::default();
    };
    Geo {
        country: city.country.iso_code.map(str::to_owned),
        region: city
            .subdivisions
            .first()
            .and_then(|subdivision| subdivision.names.english)
            .map(str::to_owned),
        city: city.city.names.english.map(str::to_owned),
    }
}

fn load(path: &Path) -> Result<(), maxminddb::MaxMindDbError> {
    // SAFETY: files are only ever replaced by renaming a new one over them, so a
    // mapped file is never modified or truncated in place.
    let reader = unsafe { Reader::open_mmap(path) }?;
    *READER
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::new(reader));
    Ok(())
}

fn is_fresh(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age < MAX_AGE)
}

/// This month's file, or last month's early in a month before DB-IP published.
async fn download(target: &Path) -> Result<(), String> {
    let now = chrono::Utc::now().date_naive();
    let last_month = now - chrono::Months::new(1);
    let client = reqwest::Client::new();
    let mut response = None;
    for month in [now, last_month] {
        let url = format!("{DOWNLOAD_URL}-{}.mmdb.gz", month.format("%Y-%m"));
        let reply = client.get(&url).send().await.map_err(|e| e.to_string())?;
        if reply.status().is_success() {
            response = Some(reply);
            break;
        }
    }
    let mut response = response.ok_or("no DB-IP city lite file for this or last month")?;

    let gz = target.with_extension("mmdb.gz.part");
    let mut file = tokio::fs::File::create(&gz)
        .await
        .map_err(|e| e.to_string())?;
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|e| e.to_string())?;
    }
    drop(file);

    let part = target.with_extension("mmdb.part");
    let (from, to) = (gz.clone(), part.clone());
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let mut decoder = flate2::read::GzDecoder::new(std::fs::File::open(&from)?);
        std::io::copy(&mut decoder, &mut std::fs::File::create(&to)?)?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    let _ = tokio::fs::remove_file(&gz).await;
    // Validate before it replaces a working file.
    // SAFETY: `part` is ours and nothing else writes it.
    unsafe { Reader::open_mmap(&part) }.map_err(|e| e.to_string())?;
    tokio::fs::rename(&part, target)
        .await
        .map_err(|e| e.to_string())
}

/// Loads the database and keeps the downloaded one fresh. Failures only log: requests
/// are still counted, just without a location.
pub fn spawn(dir: PathBuf) {
    match std::env::var("LITEPOD_GEOIP_DB")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        Some(value) if value.trim() == "off" => {}
        Some(path) => {
            if let Err(error) = load(Path::new(&path)) {
                tracing::warn!(%path, %error, "request analytics: could not open the GeoIP database");
            }
        }
        None => {
            let target = dir.join(FILE_NAME);
            tokio::spawn(async move {
                if target.exists()
                    && let Err(error) = load(&target)
                {
                    tracing::warn!(path = %target.display(), %error, "request analytics: unreadable GeoIP database, downloading again");
                }
                let mut interval = tokio::time::interval(CHECK_EVERY);
                loop {
                    interval.tick().await;
                    if is_fresh(&target) {
                        continue;
                    }
                    match download(&target).await {
                        Ok(()) => match load(&target) {
                            Ok(()) => {
                                tracing::info!(path = %target.display(), "request analytics: GeoIP database updated")
                            }
                            Err(error) => {
                                tracing::warn!(%error, "request analytics: GeoIP database unreadable after download")
                            }
                        },
                        Err(error) => {
                            tracing::warn!(%error, "request analytics: GeoIP download failed")
                        }
                    }
                }
            });
        }
    }
}
