<script lang="ts">
	import { t } from '#lib/i18n/index.svelte.js';

	let { label }: { label?: string } = $props();
</script>

<span
	class="inline-flex items-center rounded-full border border-dashed border-foreground/20 px-2 py-0.5 font-mono text-[10.5px] text-muted-foreground"
	title={t('common.preview.example_data')}>{label ?? t('common.preview.example_data')}</span>
