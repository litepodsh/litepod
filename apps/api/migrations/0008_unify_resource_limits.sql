-- Build and runtime resource limits used to be configured separately
-- (build_cpu_cores/build_memory_mib vs runtime_cpu_cores/runtime_memory_mib).
-- The code now applies a single limit to both phases, reusing the build_*
-- columns. Backfill build_* from runtime_* wherever only the runtime pair was
-- set (the common case: compose apps and DB templates could only configure
-- runtime_*, never build_*), so existing limits are not silently dropped.
-- If build_* was already set, it is kept and the old runtime_* value is
-- discarded. The runtime_cpu_cores/runtime_memory_mib columns are left in
-- place, unused — migrations are append-only, so nothing drops them.
UPDATE applications
SET build_cpu_cores = runtime_cpu_cores,
    build_memory_mib = runtime_memory_mib
WHERE build_cpu_cores IS NULL AND runtime_cpu_cores IS NOT NULL;
