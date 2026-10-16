<script lang="ts">
	import * as Popover from '#lib/components/ui/popover/index.js';
	import InfoIcon from '@lucide/svelte/icons/info';
	import { t } from '#lib/i18n/index.svelte.js';
	import { SSO_FIELD_EQUIVALENTS, type SsoHelpField } from './sso-field-help';

	let { field, title }: { field: SsoHelpField; title: string } = $props();

	const equivalents = $derived(SSO_FIELD_EQUIVALENTS[field] ?? []);
</script>

<Popover.Root>
	<Popover.Trigger
		type="button"
		class="text-muted-foreground hover:text-foreground rounded-full transition-colors"
		aria-label={t('organizations.sso.help.about', { field: title })}
		title={t('organizations.sso.help.about', { field: title })}>
		<InfoIcon class="size-3.5" />
	</Popover.Trigger>
	<Popover.Content class="w-[min(28rem,calc(100vw-2rem))] gap-3" align="start">
		<div class="grid gap-1">
			<p class="font-medium">{title}</p>
			<p class="text-xs text-muted-foreground">{t(`organizations.sso.help.${field}`)}</p>
		</div>
		{#if equivalents.length}
			<div class="grid gap-2">
				<p class="text-xs font-medium">{t('organizations.sso.help.equivalents')}</p>
				<ul class="grid gap-2 text-xs">
					{#each equivalents as row (row.idp)}
						<li class="grid gap-0.5">
							<span class="text-muted-foreground">{row.idp}</span>
							<code class="w-fit max-w-full rounded bg-muted px-1.5 py-0.5 break-all"
								>{row.name}</code>
							{#if row.where}
								<span class="text-muted-foreground">{row.where}</span>
							{/if}
						</li>
					{/each}
				</ul>
			</div>
		{/if}
	</Popover.Content>
</Popover.Root>
