//! What FreeBSD servers cannot run. Podman there executes Linux images through FreeBSD's
//! Linux ABI layer (linuxulator), which lacks a few syscalls and behaves differently in
//! others. Every case below was reproduced on FreeBSD 15.1 with Podman 5.8; deploys use it
//! to refuse up front (Railpack) or to warn in the deploy log (images).

/// Railpack builds run BuildKit inside a privileged Linux container, which needs Linux
/// namespaces and overlay mounts that the Linux layer does not provide.
pub const RAILPACK_UNSUPPORTED: &str = "Railpack builds do not run on FreeBSD servers: they need BuildKit, which requires Linux namespaces that FreeBSD's Linux layer does not provide. Add a Dockerfile to the repository and switch the build method to Dockerfile.";

const NGINX: &str = "nginx does not run on FreeBSD servers: FreeBSD's Linux layer rejects its epoll setup (epoll_ctl EINVAL), so its workers exit and the container never answers. Serve static files with a Caddy image instead.";
const POSTGRES: &str = "PostgreSQL does not run on FreeBSD servers: it needs signalfd(), which FreeBSD's Linux layer lacks, so it exits during initdb. Run the database on a Linux server.";
const MARIADB: &str = "The MariaDB image does not start on FreeBSD servers: its entrypoint reads /proc/self/cgroup, which FreeBSD's Linux layer does not provide. Use mysql with user 999:999, or a Linux server.";
const DROPS_ROOT: &str = "This image's entrypoint drops root with setpriv or gosu, which FreeBSD's Linux layer refuses, so it exits at once. Run it as its own user instead: 999:999 for the official redis and mysql images.";
const MUSL_NODE: &str = "Node.js 20 and later on Alpine hang at startup on FreeBSD servers: musl probes the stack with mremap, which FreeBSD's Linux layer answers with ENOMEM instead of EFAULT, so the probe never ends. Use a Debian-based tag such as node:22-slim.";

/// The known FreeBSD problem with `reference`, if any. `runs_as_user` is whether the
/// container already runs as a non-root user, which avoids the privilege-drop failure.
pub fn image_issue(reference: &str, runs_as_user: bool) -> Option<&'static str> {
    let (name, tag) = name_and_tag(reference);
    match name {
        "nginx" | "nginx-unprivileged" | "openresty" => Some(NGINX),
        "postgres" | "postgresql" | "postgis" | "timescaledb" | "timescaledb-ha" | "pgvector" => {
            Some(POSTGRES)
        }
        "mariadb" => Some(MARIADB),
        "redis" | "mysql" if !runs_as_user => Some(DROPS_ROOT),
        "node" if is_alpine_node_20_or_later(tag) => Some(MUSL_NODE),
        _ => None,
    }
}

/// Known problems with the base images a Dockerfile builds from, as `(image, problem)`.
/// Only plain `FROM` lines count; a stage built `FROM` an earlier stage name is skipped
/// because its image is checked where that stage is declared.
pub fn dockerfile_issues(dockerfile: &str) -> Vec<(String, &'static str)> {
    let mut stages = Vec::new();
    let mut issues = Vec::new();
    for line in dockerfile.lines() {
        let mut words = line.split_whitespace();
        if !words
            .next()
            .is_some_and(|word| word.eq_ignore_ascii_case("from"))
        {
            continue;
        }
        let Some(image) = words.find(|word| !word.starts_with("--")) else {
            continue;
        };
        let from_stage = stages.contains(&image.to_ascii_lowercase());
        if let (Some(keyword), Some(stage)) = (words.next(), words.next())
            && keyword.eq_ignore_ascii_case("as")
        {
            stages.push(stage.to_ascii_lowercase());
        }
        if from_stage {
            continue;
        }
        if let Some(issue) = image_issue(image, false)
            && !issues.iter().any(|(_, known)| *known == issue)
        {
            issues.push((image.to_string(), issue));
        }
    }
    issues
}

/// Prints what [`meminfo_from_sysctl`] reads: FreeBSD has no `/proc/meminfo`. `-i` skips
/// the ZFS counters on a host without ZFS instead of failing.
pub const MEMINFO_SCRIPT: &str = "sysctl -i hw.pagesize vm.stats.vm.v_free_count vm.stats.vm.v_inactive_count kstat.zfs.misc.arcstats.size kstat.zfs.misc.arcstats.c_min; swapctl -sk";

/// The `MemAvailable:` and `SwapFree:` lines of `/proc/meminfo`, built from the output of
/// [`MEMINFO_SCRIPT`] so the Linux parsers read FreeBSD too. Available memory is the free
/// and inactive pages plus the ZFS cache above its floor, which the kernel hands back
/// under pressure the way Linux drops its page cache.
pub fn meminfo_from_sysctl(output: &str) -> Option<String> {
    let value = |key: &str| {
        output.lines().find_map(|line| {
            line.strip_prefix(key)?
                .strip_prefix(':')?
                .trim()
                .parse::<u64>()
                .ok()
        })
    };
    let page = value("hw.pagesize")?;
    let pages = value("vm.stats.vm.v_free_count")? + value("vm.stats.vm.v_inactive_count")?;
    let arc = match (
        value("kstat.zfs.misc.arcstats.size"),
        value("kstat.zfs.misc.arcstats.c_min"),
    ) {
        (Some(size), Some(min)) => size.saturating_sub(min),
        _ => 0,
    };
    let available_kb = (pages.saturating_mul(page) + arc) / 1024;
    // `swapctl -sk`: `Total:  <size KiB>  <used KiB>`.
    let swap_free_kb = output
        .lines()
        .find_map(|line| {
            let mut words = line.strip_prefix("Total:")?.split_whitespace();
            let total = words.next()?.parse::<u64>().ok()?;
            let used = words.next()?.parse::<u64>().ok()?;
            Some(total.saturating_sub(used))
        })
        .unwrap_or(0);
    Some(format!(
        "MemAvailable: {available_kb} kB\nSwapFree: {swap_free_kb} kB\n"
    ))
}

/// This host's memory as `/proc/meminfo` text, on Linux or FreeBSD.
pub async fn local_meminfo() -> Option<String> {
    if cfg!(target_os = "freebsd") {
        let output = tokio::process::Command::new("/bin/sh")
            .args(["-c", MEMINFO_SCRIPT])
            .output()
            .await
            .ok()?;
        meminfo_from_sysctl(&String::from_utf8_lossy(&output.stdout))
    } else {
        tokio::fs::read_to_string("/proc/meminfo").await.ok()
    }
}

/// `docker.io/library/node:22-alpine@sha256:…` → `("node", "22-alpine")`. The tag is
/// empty when the reference has none.
fn name_and_tag(reference: &str) -> (&str, &str) {
    let reference = reference.split('@').next().unwrap_or(reference);
    let last = reference.rsplit('/').next().unwrap_or(reference);
    last.split_once(':').unwrap_or((last, ""))
}

/// `alpine`, `alpine3.20`, `lts-alpine` and `22-alpine` are; `18-alpine` and `22-slim`
/// are not. Node 18 does not probe the main thread's stack, so it starts.
fn is_alpine_node_20_or_later(tag: &str) -> bool {
    if !tag.contains("alpine") {
        return false;
    }
    let major: String = tag.chars().take_while(char::is_ascii_digit).collect();
    major.parse::<u32>().map_or(true, |major| major >= 20)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_issue_names_the_images_that_fail_under_the_linux_layer() {
        assert_eq!(image_issue("nginx:alpine", false), Some(NGINX));
        assert_eq!(image_issue("docker.io/library/nginx:1.27", false), Some(NGINX));
        assert_eq!(image_issue("docker.io/library/postgres:17.2", false), Some(POSTGRES));
        assert_eq!(image_issue("mariadb:11.8", false), Some(MARIADB));
        assert_eq!(image_issue("redis:7-alpine", false), Some(DROPS_ROOT));
        assert_eq!(image_issue("redis:7", true), None);
        assert_eq!(image_issue("node:22-alpine", false), Some(MUSL_NODE));
        assert_eq!(image_issue("node:alpine", false), Some(MUSL_NODE));
        assert_eq!(image_issue("node:18-alpine", false), None);
        assert_eq!(image_issue("node:22-slim", false), None);
        assert_eq!(image_issue("caddy:2-alpine", false), None);
        assert_eq!(image_issue("registry.local:5000/team/app:1", false), None);
        assert_eq!(
            image_issue("node@sha256:0123456789abcdef", false),
            None,
            "a digest-only reference has no tag to read"
        );
    }

    #[test]
    fn dockerfile_issues_reads_from_lines_and_skips_stage_references() {
        let dockerfile = "\
FROM --platform=linux/amd64 node:22-alpine AS build
RUN npm ci
from build as test
FROM nginx:1.27
COPY --from=build /app/dist /usr/share/nginx/html
";
        assert_eq!(
            dockerfile_issues(dockerfile),
            vec![
                ("node:22-alpine".to_string(), MUSL_NODE),
                ("nginx:1.27".to_string(), NGINX),
            ]
        );
        assert!(dockerfile_issues("FROM node:22-slim\nCMD [\"node\"]\n").is_empty());
    }

    #[test]
    fn meminfo_from_sysctl_counts_free_inactive_and_reclaimable_zfs_cache() {
        let output = "\
hw.pagesize: 4096
vm.stats.vm.v_free_count: 454435
vm.stats.vm.v_inactive_count: 55452
kstat.zfs.misc.arcstats.size: 484130560
kstat.zfs.misc.arcstats.c_min: 131120000
Total:           2097152      46936
";
        assert_eq!(
            meminfo_from_sysctl(output).as_deref(),
            Some("MemAvailable: 2384284 kB\nSwapFree: 2050216 kB\n")
        );
        assert_eq!(
            meminfo_from_sysctl("hw.pagesize: 4096\nvm.stats.vm.v_free_count: 256\nvm.stats.vm.v_inactive_count: 0\n").as_deref(),
            Some("MemAvailable: 1024 kB\nSwapFree: 0 kB\n"),
            "a host without ZFS or swap still reports its free pages"
        );
        assert_eq!(meminfo_from_sysctl("sysctl: unknown oid"), None);
    }
}
