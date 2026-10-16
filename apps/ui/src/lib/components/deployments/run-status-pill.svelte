<script lang="ts">
	import { toneClass, type StatusTone } from '#lib/utils/status-tone.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let { status }: { status: string } = $props();

	const tone = $derived<StatusTone>(
		status === 'succeeded'
			? 'ok'
			: status === 'failed'
				? 'danger'
				: status === 'building' || status === 'deploying'
					? 'warn'
					: status === 'queued' || status === 'cancel_requested'
						? 'info'
						: 'neutral',
	);
	const label = $derived.by(() => {
		const key = `projects.deployment_log_viewer.status_${status}`;
		const value = t(key);
		return value === key ? status.replace('_', ' ') : value;
	});
	const active = $derived(status === 'building' || status === 'deploying');
</script>

<span
	class="inline-flex h-[22px] shrink-0 items-center gap-1.5 rounded-full px-2 text-xs font-medium whitespace-nowrap lp-raised {toneClass(
		tone,
	)}">
	<span class="size-1.5 shrink-0 rounded-full bg-current {active ? 'pip-pulse' : ''}"></span>
	{label}
</span>

<style>
	.pip-pulse {
		animation: pip 1.2s var(--lp-ease) infinite;
	}
	@keyframes pip {
		50% {
			opacity: 0.35;
			transform: scale(0.7);
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.pip-pulse {
			animation: none;
		}
	}
</style>
