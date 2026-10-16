use chrono::Utc;
use podman_api::{
    models,
    opts::{ContainerCreateOpts, ContainerCreateOptsBuilder, ContainerRestartPolicy},
};
use sqlx::Row;

use crate::{
    dtos::{deployment::SourceConfigDto, response::AppError},
    state::state::SharedState,
};

pub(super) const MASK: &str = "••••••";

pub(super) const CONTAINER_PORT: i64 = 80;

pub(crate) const BUILD_CPU_PERIOD_US: isize = 100_000;

pub(super) const BYTES_PER_MIB: u64 = 1024 * 1024;

/// Podman's build and container APIs use different option shapes, so normalize
/// the persisted human units once before handing them to either backend. This
/// single limit is applied to both the build step and the running
/// container/service — there used to be a separate `runtime_*` pair, but
/// nothing needed them to differ and it doubled every code path (and config
/// form) that touched resource limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResourceLimits {
    pub cpu_quota_us: isize,
    pub memory_bytes: usize,
    pub memswap_bytes: usize,
}

pub(crate) fn resource_limits(source: &SourceConfigDto) -> Result<Option<ResourceLimits>, String> {
    let (cpu, memory_mib) = match (source.build_cpu_cores, source.build_memory_mib) {
        (None, None) => return Ok(None),
        (Some(cpu), Some(memory_mib)) => (cpu, memory_mib),
        _ => return Err("Resource limits must include both CPU and memory".into()),
    };
    if !cpu.is_finite() || cpu < 0.1 || memory_mib <= 0 {
        return Err("Resource limits must include a positive CPU and memory value".into());
    }
    let quota = (cpu * BUILD_CPU_PERIOD_US as f64).round();
    if quota > isize::MAX as f64 {
        return Err("CPU limit is too large".into());
    }
    let memory_bytes = u64::try_from(memory_mib)
        .ok()
        .and_then(|mib| mib.checked_mul(BYTES_PER_MIB))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or_else(|| "Memory limit is too large".to_string())?;
    let swap_extra_bytes = u64::try_from(source.build_swap_mib.unwrap_or(0).max(0))
        .ok()
        .and_then(|mib| mib.checked_mul(BYTES_PER_MIB))
        .ok_or_else(|| "Swap limit is too large".to_string())?;
    let memswap_bytes = memory_bytes
        .checked_add(usize::try_from(swap_extra_bytes).unwrap_or(usize::MAX))
        .ok_or_else(|| "Swap limit is too large".to_string())?;
    Ok(Some(ResourceLimits {
        cpu_quota_us: quota as isize,
        memory_bytes,
        memswap_bytes,
    }))
}

pub(crate) fn resource_log_line(source: &SourceConfigDto) -> String {
    match (source.build_cpu_cores, source.build_memory_mib) {
        (Some(cpu), Some(memory_mib)) => {
            let swap = source.build_swap_mib.unwrap_or(0).max(0);
            if swap > 0 {
                format!(
                    "Resources: limited to {cpu} CPU cores, {memory_mib} MiB RAM + {swap} MiB swap"
                )
            } else {
                format!("Resources: limited to {cpu} CPU cores and {memory_mib} MiB RAM")
            }
        }
        _ => "Resources: no CPU/RAM limit configured".into(),
    }
}

/// `ContainerCreateOptsBuilder` takes different units than the image build opts:
/// the CPU pair is `u64`/`i64` and memory only travels inside `resource_limits`.
pub(crate) fn with_resource_limits(
    mut builder: ContainerCreateOptsBuilder,
    source: &SourceConfigDto,
) -> Result<ContainerCreateOptsBuilder, String> {
    if let Some(limits) = resource_limits(source)? {
        let memory = i64::try_from(limits.memory_bytes)
            .map_err(|_| "Memory limit is too large".to_string())?;
        let memswap = i64::try_from(limits.memswap_bytes)
            .map_err(|_| "Swap limit is too large".to_string())?;
        builder = builder
            .cpu_period(BUILD_CPU_PERIOD_US as u64)
            .cpu_quota(limits.cpu_quota_us as i64)
            .resource_limits(models::LinuxResources {
                block_io: None,
                cpu: None,
                devices: None,
                hugepage_limits: None,
                memory: Some(models::LinuxMemory {
                    disable_oom_killer: None,
                    kernel: None,
                    kernel_tcp: None,
                    limit: Some(memory),
                    reservation: None,
                    // Previously reused `memory` here, so a configured swap
                    // limit never actually reached the running container.
                    swap: Some(memswap),
                    swappiness: None,
                    use_hierarchy: None,
                }),
                network: None,
                pids: None,
                rdma: None,
                unified: None,
            });
    }
    Ok(builder)
}

pub(crate) fn managed_container_builder(
    name: &str,
    image: String,
    restart_policy: &str,
) -> ContainerCreateOptsBuilder {
    let policy = match restart_policy {
        "always" => ContainerRestartPolicy::Always,
        "on-failure" => ContainerRestartPolicy::OnFailure,
        "unless-stopped" => ContainerRestartPolicy::UnlessStopped,
        _ => ContainerRestartPolicy::No,
    };
    ContainerCreateOpts::builder()
        .name(name)
        .image(image)
        .restart_policy(policy)
}

pub(super) fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub(super) fn short_application_suffix(application_id: &str) -> &str {
    let final_uuid_section = application_id.rsplit('-').next().unwrap_or(application_id);
    final_uuid_section.get(..6).unwrap_or(final_uuid_section)
}

/// Port Caddy dials inside the container: the app's configured port for git
/// deploys, the caddy file-server port for static ones.
pub(crate) async fn app_container_port(
    state: &SharedState,
    application_id: &str,
) -> Result<i64, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let Some(row) =
        sqlx::query("SELECT deploy_type, container_port FROM applications WHERE id = ?1")
            .bind(application_id)
            .fetch_optional(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
    else {
        return Ok(CONTAINER_PORT);
    };
    let deploy_type: String = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let port: Option<i64> = row
        .try_get(1)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(if deploy_type == "git" {
        port.unwrap_or(CONTAINER_PORT)
    } else {
        CONTAINER_PORT
    })
}
