<script lang="ts">
	import type { DeploymentJobDto } from '#lib/clients/project-client.js';
	import { toneClass, toneDot, type StatusTone } from '#lib/utils/status-tone.js';
	import { relativeTime } from '#lib/utils/relative-time.js';
	import { i18n } from '#lib/i18n/index.svelte.js';

	let { job }: { job: DeploymentJobDto } = $props();

	function tone(status: DeploymentJobDto['status']): StatusTone {
		switch (status) {
			case 'succeeded':
				return 'ok';
			case 'failed':
				return 'danger';
			case 'building':
			case 'deploying':
				return 'warn';
			default:
				return 'neutral';
		}
	}
</script>

<li
	class="grid grid-cols-[8px_minmax(0,1fr)_auto] items-start gap-2.5 rounded-lg px-2.5 py-2 transition-colors hover:bg-muted">
	<span
		class="mt-1.5 size-2 rounded-full {toneDot(tone(job.status))} {job.status === 'building' ||
		job.status === 'deploying'
			? 'motion-safe:animate-pulse'
			: ''}"></span>
	<span class="min-w-0">
		<b class="block truncate text-sm font-medium">{job.application_name}</b>
		<small class="block truncate text-xs text-muted-foreground"
			>{job.project_name} ·
			<time
				datetime={new Date(job.queued_at).toISOString()}
				title={new Date(job.queued_at).toLocaleString()}
				>{relativeTime(job.queued_at, Date.now(), i18n.locale)}</time
			></small>
	</span>
	<span class="rounded-full px-2 py-0.5 text-xs font-medium {toneClass(tone(job.status))}"
		>{job.status}</span>
</li>
