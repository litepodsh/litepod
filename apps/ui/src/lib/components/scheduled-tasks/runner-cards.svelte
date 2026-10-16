<script lang="ts" module>
	import BoxIcon from '@lucide/svelte/icons/box';
	import ContainerIcon from '@lucide/svelte/icons/container';
	import TerminalIcon from '@lucide/svelte/icons/terminal';
	import WebhookIcon from '@lucide/svelte/icons/webhook';
	import type { ScheduledTaskRunner } from '#lib/clients/scheduled-tasks-client.js';

	export const RUNNER_ICON = {
		ephemeral: ContainerIcon,
		application_exec: BoxIcon,
		remote_shell: TerminalIcon,
		litepod_api: WebhookIcon,
	} satisfies Record<ScheduledTaskRunner, unknown>;
</script>

<script lang="ts">
	import { t } from '#lib/i18n/index.svelte.js';
	import { cn } from '#lib/utils.js';

	let {
		value = $bindable('ephemeral'),
		runners,
		locked = false,
		onchange,
	}: {
		value?: ScheduledTaskRunner;
		/** Runners this user may pick; litepod_api only for sysadmins outside cloud. */
		runners: ScheduledTaskRunner[];
		/** Opened from an app: only application_exec stays selectable. */
		locked?: boolean;
		onchange?: (runner: ScheduledTaskRunner) => void;
	} = $props();
</script>

<div
	role="radiogroup"
	aria-label={t('organizations.task_ui.where_runs')}
	class="grid grid-cols-1 gap-2 sm:grid-cols-2">
	{#each runners as runner (runner)}
		{@const Icon = RUNNER_ICON[runner]}
		{@const on = runner === value}
		{@const disabled = locked && runner !== 'application_exec'}
		<button
			type="button"
			role="radio"
			aria-checked={on}
			{disabled}
			class={cn(
				'lp-raised relative grid grid-cols-[2rem_minmax(0,1fr)] items-start gap-2.5 rounded-xl border bg-card p-3 pe-8 text-left transition-[border-color,background-color,transform] duration-200 ease-[var(--lp-ease)] hover:border-muted-foreground/50 active:scale-[.99] disabled:pointer-events-none disabled:opacity-50 motion-reduce:transition-none',
				on && 'border-primary bg-primary/8 ring-3 ring-primary/12 hover:border-primary',
			)}
			onclick={() => {
				value = runner;
				onchange?.(runner);
			}}>
			<span
				class={cn(
					'lp-raised grid size-8 place-items-center rounded-[9px]',
					on ? 'bg-primary text-primary-foreground' : 'bg-muted text-muted-foreground',
				)}><Icon class="size-4" /></span>
			<span class="min-w-0">
				<b class="block text-[13px] font-semibold">{t(`organizations.task_ui.runner_${runner}`)}</b>
				<small class="mt-0.5 block text-xs leading-snug text-muted-foreground"
					>{t(`organizations.task_ui.runner_${runner}_desc`)}</small>
			</span>
			<span
				aria-hidden="true"
				class={cn(
					'lp-well absolute top-2.5 right-2.5 size-4 rounded-full',
					on
						? 'bg-primary shadow-[inset_0_0_0_4px_var(--primary),inset_0_0_0_8px_#fff]'
						: 'bg-background',
				)}></span>
		</button>
	{/each}
</div>
