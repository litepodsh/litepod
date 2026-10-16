<script lang="ts" generics="T extends string">
	// Reusable shell picker: a shadcn Select whose trigger and items each show a
	// ShellIcon + label. Callers pass their own option set (bare names like
	// `bash` or full paths like `/bin/bash`); `format` controls the label.
	import * as Select from '#lib/components/ui/select/index.js';
	import ShellIcon from './shell-icon.svelte';

	let {
		value = $bindable(),
		options,
		onValueChange,
		id,
		disabled = false,
		triggerClass = 'font-mono',
		format = ((v: string) => v.replace(/^\/(?:usr\/)?bin\//, '')) as (value: T) => string,
	}: {
		value: T;
		options: readonly T[];
		onValueChange?: (value: T) => void;
		id?: string;
		disabled?: boolean;
		triggerClass?: string;
		format?: (value: T) => string;
	} = $props();
</script>

<Select.Root
	type="single"
	{value}
	{disabled}
	onValueChange={(next) => {
		value = next as T;
		onValueChange?.(next as T);
	}}>
	<Select.Trigger {id} size="sm" class={`gap-1.5 ${triggerClass}`}>
		<ShellIcon shell={value} class="size-3.5 shrink-0" />
		{format(value)}
	</Select.Trigger>
	<Select.Content>
		{#each options as option (option)}
			<Select.Item value={option}>
				<ShellIcon shell={option} class="size-3.5 shrink-0" />
				{format(option)}
			</Select.Item>
		{/each}
	</Select.Content>
</Select.Root>
