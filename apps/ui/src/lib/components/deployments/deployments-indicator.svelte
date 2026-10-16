<script lang="ts">
	import { onMount } from 'svelte';
	import { fly } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import XIcon from '@lucide/svelte/icons/x';
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';
	import * as Popover from '#lib/components/ui/popover/index.js';
	import LiveTimer from '#lib/components/live-timer.svelte';
	import { listOrganizations } from '#lib/clients/organization-client.js';
	import { resolveSelectedOrganization } from '#lib/utils/selected-organization.js';
	import {
		listDeploymentQueue,
		streamDeploymentQueueEvents,
		type DeploymentJobDto,
	} from '#lib/clients/project-client.js';
	import { invalidateOnboarding } from '#lib/state/onboarding.svelte.js';
	import { t } from '#lib/i18n/index.svelte.js';

	const ACTIVE = ['queued', 'building', 'deploying'];
	const isActive = (job: DeploymentJobDto) => ACTIVE.includes(job.status);

	let orgId = $state<string | null>(null);
	let jobs = $state<DeploymentJobDto[]>([]);
	let dismissedIds = $state<Set<string>>(new Set());

	const activeJobs = $derived(jobs.filter(isActive));
	const visibleJobs = $derived(activeJobs.filter((job) => !dismissedIds.has(job.id)));

	async function refresh() {
		if (!orgId) return;
		try {
			jobs = await listDeploymentQueue(orgId, 50);
		} catch {
			return;
		}
		// Keep the dismiss set from growing without bound.
		const stillActive = new Set(jobs.filter(isActive).map((job) => job.id));
		if ([...dismissedIds].some((id) => !stillActive.has(id))) {
			dismissedIds = new Set([...dismissedIds].filter((id) => stillActive.has(id)));
		}
	}

	function dismiss() {
		dismissedIds = new Set(activeJobs.map((job) => job.id));
	}

	function statusLabel(status: string) {
		return status.charAt(0).toUpperCase() + status.slice(1);
	}

	onMount(() => {
		let events: EventSource | null = null;
		const update = () => void refresh();
		const terminal = () => {
			void refresh();
			invalidateOnboarding();
		};

		void (async () => {
			try {
				orgId = resolveSelectedOrganization(await listOrganizations())?.id ?? null;
			} catch {
				orgId = null;
			}
			if (!orgId) return;
			await refresh();
			events = streamDeploymentQueueEvents();
			events.addEventListener('status', update);
			events.addEventListener('terminal', terminal);
		})();

		return () => {
			events?.removeEventListener('status', update);
			events?.removeEventListener('terminal', terminal);
			events?.close();
		};
	});
</script>

{#if visibleJobs.length > 0}
	<div class="px-1 group-data-[collapsible=icon]:hidden">
		<div class="group relative" transition:fly={{ y: 8, duration: 250, easing: cubicOut }}>
			<Popover.Root>
				<Popover.Trigger
					class="border-border bg-background hover:bg-accent focus-visible:ring-ring inline-flex items-center gap-2 rounded-full border px-3 py-1.5 text-sm shadow-sm outline-none transition-colors focus-visible:ring-2">
					<LoaderCircleIcon class="text-muted-foreground size-4 animate-spin" />
					<span class="font-medium tabular-nums">{visibleJobs.length}</span>
					<span class="text-muted-foreground">{t('common.deployments.deploying')}</span>
				</Popover.Trigger>
				<Popover.Content side="top" align="start" class="w-72 overflow-hidden p-0">
					<div class="flex items-center justify-between px-3 py-2">
						<span class="text-sm font-medium">{t('common.deployments.in_progress')}</span>
						<span class="text-muted-foreground text-xs tabular-nums">{visibleJobs.length}</span>
					</div>
					<div class="border-t">
						{#each visibleJobs as job (job.id)}
							<a
								href={`/projects/${encodeURIComponent(job.project_id)}/environments/${encodeURIComponent(job.environment_id)}/applications/${encodeURIComponent(job.application_id)}?tab=deployment`}
								class="hover:bg-accent flex items-center gap-2 px-3 py-2 text-sm">
								<LoaderCircleIcon class="text-muted-foreground size-3.5 shrink-0 animate-spin" />
								<span class="min-w-0 flex-1">
									<span class="block truncate font-medium">{job.application_name}</span>
									<span class="text-muted-foreground block truncate text-xs"
										>{job.project_name}</span>
								</span>
								<span class="text-muted-foreground shrink-0 text-xs tabular-nums">
									{statusLabel(job.status)} · <LiveTimer
										startedAt={job.started_at ?? job.queued_at} />
								</span>
							</a>
						{/each}
					</div>
					{#if orgId}
						<a
							href={`/organizations/${encodeURIComponent(orgId)}/deployment-queue`}
							class="hover:bg-accent text-muted-foreground flex items-center justify-between border-t px-3 py-2 text-xs">
							{t('common.deployments.view_queue')}
							<ArrowRightIcon class="size-3.5" />
						</a>
					{/if}
				</Popover.Content>
			</Popover.Root>
			<button
				type="button"
				onclick={dismiss}
				aria-label={t('common.assistant.dismiss')}
				class="border-border bg-background text-muted-foreground hover:text-foreground absolute -top-2 -right-2 grid size-5 place-items-center rounded-full border opacity-0 shadow-sm transition-opacity group-hover:opacity-100">
				<XIcon class="size-3" />
			</button>
		</div>
	</div>
{/if}
