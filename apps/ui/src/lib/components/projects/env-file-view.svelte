<script module lang="ts">
	const NAME = /^[A-Z_][A-Z0-9_]*$/;

	/**
	 * Mirrors the API's `parse_environment`: blank and `#` lines are skipped, every
	 * other line is `NAME=value` with an upper-case name, and a name appears once.
	 * `errors` holds 1-based line numbers the API would reject.
	 */
	export function parseEnv(text: string) {
		const vars: Record<string, string> = {};
		const errors: number[] = [];
		text.split('\n').forEach((raw, index) => {
			const line = raw.trim();
			if (!line || line.startsWith('#')) return;
			const at = line.indexOf('=');
			const name = at < 0 ? '' : line.slice(0, at);
			if (!NAME.test(name) || name in vars) errors.push(index + 1);
			else vars[name] = line.slice(at + 1);
		});
		return { vars, errors };
	}
</script>

<script lang="ts">
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		vars,
		reveal = false,
		domains = [],
		view = 'file',
	}: {
		vars: Record<string, string>;
		reveal?: boolean;
		/** Bound domains and their markers; a value containing one is "linked". */
		domains?: string[];
		view?: 'file' | 'table';
	} = $props();

	// Fixed width so a masked line never hints at the secret's length.
	const MASK = '••••••••••';

	const rows = $derived(
		Object.entries(vars).map(([key, value]) => ({
			key,
			value,
			linked: domains.some((domain) => domain && value.includes(domain)),
		})),
	);
</script>

{#if view === 'table'}
	{#if rows.length}
		<div class="overflow-x-auto">
			<table class="w-full text-sm">
				<thead>
					<tr class="text-left text-xs text-muted-foreground *:lp-groove-b">
						<th class="px-4 py-2 font-medium">{t('projects.application_env_tab.col_key')}</th>
						<th class="px-4 py-2 font-medium">{t('projects.application_env_tab.col_value')}</th>
						<th class="px-4 py-2"></th>
					</tr>
				</thead>
				<tbody class="[&>tr:last-child>td]:border-0 [&>tr:last-child>td]:shadow-none">
					{#each rows as row (row.key)}
						<tr class="*:lp-groove-b">
							<td class="px-4 py-2.5 font-mono text-xs font-semibold break-all">{row.key}</td>
							<td class="px-4 py-2.5 font-mono text-xs break-all">
								{#if reveal || row.linked}
									<span class={row.linked ? 'linked text-success' : ''}>{row.value}</span>
								{:else}
									<span class="text-muted-foreground tracking-widest">{MASK}</span>
								{/if}
							</td>
							<td class="px-4 py-2.5 text-right">
								{#if row.linked}
									<span
										class="rounded-full bg-success/10 px-2 py-0.5 font-mono text-[11px] text-success"
										>{t('projects.application_env_tab.domain_badge')}</span>
								{/if}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{:else}
		<p class="px-4 py-6 text-sm text-muted-foreground">
			{t('projects.application_env_tab.empty_scope')}
		</p>
	{/if}
{:else}
	<div
		class="envv lp-well grid max-h-[520px] grid-cols-[44px_minmax(0,1fr)] overflow-auto bg-muted/60 font-mono text-[12.5px] leading-[1.75] max-sm:text-xs">
		<pre
			class="m-0 border-r py-3 pr-2.5 text-right text-muted-foreground/70 select-none"
			aria-hidden="true">{#each rows.length ? rows : [null] as _, index (index)}<span
					class="ln block">{index + 1}</span
				>{/each}</pre>
		<pre
			class="code m-0 min-w-0 overflow-x-auto py-3 pr-4 pl-1.5">{#if rows.length}{#each rows as row (row.key)}<span
						class="block"
						><span class="k text-info">{row.key}</span><span class="op text-muted-foreground"
							>=</span
						>{#if reveal || row.linked}<span class={row.linked ? 'v linked text-success' : 'v'}
								>{row.value}</span
							>{:else}<span class="m tracking-widest text-muted-foreground">{MASK}</span>{/if}</span
					>{/each}{:else}<span class="block text-muted-foreground italic"
					># {t('projects.application_env_tab.empty_scope')}</span
				>{/if}</pre>
	</div>
{/if}
