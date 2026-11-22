-- Runtime limits come back as an optional override of the shared build_*
-- limits, now that a build server can build an application somewhere other
-- than where it runs. 0008 left runtime_cpu_cores/runtime_memory_mib in place
-- with stale values it had discarded; clear them so nothing reappears as a
-- limit nobody set. NULL means "use the build_* limits", as before.
UPDATE applications SET runtime_cpu_cores = NULL, runtime_memory_mib = NULL;
