<script lang="ts">
	import type { ScheduledTaskShell } from '#lib/clients/scheduled-tasks-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { cn } from '#lib/utils.js';
	import ShellIcon from '#lib/components/shell-icon.svelte';

	let { value = $bindable('bash') }: { value?: ScheduledTaskShell } = $props();

	const SHELLS: ScheduledTaskShell[] = ['sh', 'bash', 'zsh', 'fish', 'nu'];
	const index = $derived(Math.max(0, SHELLS.indexOf(value)));
</script>

<div
	role="radiogroup"
	aria-label={t('organizations.tasks.shell')}
	class="lp-well relative grid grid-cols-5 rounded-xl bg-muted p-[3px]">
	<span
		aria-hidden="true"
		class="lp-raised absolute inset-y-[3px] left-[3px] rounded-[9px] bg-card transition-transform duration-450 ease-[cubic-bezier(.3,1.25,.4,1)] motion-reduce:transition-none"
		style="width: calc((100% - 6px) / 5); transform: translateX({index * 100}%)"></span>
	{#each SHELLS as shell (shell)}
		{@const selected = shell === value}
		<button
			type="button"
			role="radio"
			aria-checked={selected}
			class={cn(
				'relative z-1 flex min-w-0 flex-col items-center gap-px rounded-[9px] px-1 py-1.5 hover:text-foreground',
				selected ? 'text-foreground' : 'text-muted-foreground',
			)}
			onclick={() => (value = shell)}>
			<b class="flex items-center gap-1 font-mono text-[12.5px] font-medium"
				><ShellIcon {shell} class="size-3.5 shrink-0" />{shell}</b>
			<small class="truncate text-[10.5px] text-muted-foreground max-sm:hidden"
				>{t(`organizations.task_ui.shell_${shell}`)}</small>
		</button>
	{/each}
</div>
