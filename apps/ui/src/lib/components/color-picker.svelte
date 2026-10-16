<script lang="ts">
	import * as Popover from '#lib/components/ui/popover/index.js';
	import { Slider } from '#lib/components/ui/slider/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { cn } from '#lib/utils.js';
	import { hexToHsv, hsvToHex, isValidHex, normalizeHex, type Hsv } from '#lib/utils/color.js';
	import PipetteIcon from '@lucide/svelte/icons/pipette';

	let {
		value = $bindable(''),
		presets = [
			'#ef4444',
			'#f97316',
			'#eab308',
			'#22c55e',
			'#06b6d4',
			'#3b82f6',
			'#8b5cf6',
			'#ec4899',
			'#64748b',
			'#000000',
		],
		showHue = true,
		showInputs = true,
		showEyeDropper = true,
		showPresets = true,
		class: className,
	}: {
		value?: string;
		presets?: string[];
		showHue?: boolean;
		showInputs?: boolean;
		showEyeDropper?: boolean;
		showPresets?: boolean;
		class?: string;
	} = $props();

	let open = $state(false);
	let dragging = $state(false);
	let hsv = $state<Hsv>(hexToHsv(value || '#000000'));
	let hexDraft = $state(value ? normalizeHex(value) : '');
	let svEl: HTMLDivElement | null = $state(null);

	$effect(() => {
		if (!dragging && isValidHex(value || '')) {
			hsv = hexToHsv(value);
			hexDraft = normalizeHex(value);
		}
	});

	function commit(next: Hsv) {
		hsv = next;
		const hex = hsvToHex(next);
		value = hex;
		hexDraft = hex;
	}

	function updateFromPointer(e: PointerEvent) {
		if (!svEl) return;
		const rect = svEl.getBoundingClientRect();
		const x = Math.min(Math.max(e.clientX - rect.left, 0), rect.width);
		const y = Math.min(Math.max(e.clientY - rect.top, 0), rect.height);
		commit({ h: hsv.h, s: (x / rect.width) * 100, v: 100 - (y / rect.height) * 100 });
	}

	function onSvPointerDown(e: PointerEvent) {
		dragging = true;
		(e.target as HTMLElement).setPointerCapture(e.pointerId);
		updateFromPointer(e);
	}

	function onSvPointerMove(e: PointerEvent) {
		if (dragging) updateFromPointer(e);
	}

	function onSvPointerUp() {
		dragging = false;
	}

	function onHueChange(h: number | undefined) {
		if (h === undefined) return;
		commit({ ...hsv, h });
	}

	function onHexInput(raw: string) {
		hexDraft = raw;
		const candidate = raw.startsWith('#') ? raw : `#${raw}`;
		if (isValidHex(candidate)) {
			hsv = hexToHsv(candidate);
			value = normalizeHex(candidate);
		}
	}

	async function useEyeDropper() {
		const EyeDropperCtor = (
			window as unknown as { EyeDropper?: new () => { open(): Promise<{ sRGBHex: string }> } }
		).EyeDropper;
		if (!EyeDropperCtor) return;
		try {
			const result = await new EyeDropperCtor().open();
			onHexInput(result.sRGBHex);
		} catch {
			// user cancelled the eyedropper
		}
	}

	const hasEyeDropper = typeof window !== 'undefined' && 'EyeDropper' in window;
	const previewHex = $derived(value && isValidHex(value) ? normalizeHex(value) : 'transparent');
</script>

<Popover.Root bind:open>
	<Popover.Trigger
		class={cn(
			'flex size-9 shrink-0 items-center justify-center rounded-md border transition-shadow hover:ring-2 hover:ring-ring/30',
			className,
		)}
		style="background-color: {previewHex}"
		aria-label="Pick color"></Popover.Trigger>
	<Popover.Content class="w-64 gap-3">
		<div
			bind:this={svEl}
			role="slider"
			aria-label="Saturation and brightness"
			aria-valuenow={Math.round(hsv.s)}
			tabindex="0"
			class="relative h-36 w-full cursor-crosshair touch-none rounded-lg select-none"
			style="background-color: hsl({hsv.h}, 100%, 50%); background-image: linear-gradient(to top, #000, transparent), linear-gradient(to right, #fff, transparent);"
			onpointerdown={onSvPointerDown}
			onpointermove={onSvPointerMove}
			onpointerup={onSvPointerUp}>
			<div
				class="pointer-events-none absolute size-3.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-white shadow ring-1 ring-black/20"
				style="left: {hsv.s}%; top: {100 - hsv.v}%; background-color: {hsvToHex(hsv)}">
			</div>
		</div>

		{#if showHue}
			<div class="flex items-center gap-2">
				<div class="h-9 w-9 shrink-0 rounded-md border" style="background-color: {hsvToHex(hsv)}">
				</div>
				<Slider
					type="single"
					value={hsv.h}
					onValueChange={onHueChange}
					min={0}
					max={360}
					step={1}
					class="[&_[data-slot=slider-track]]:bg-[linear-gradient(to_right,red,yellow,lime,cyan,blue,magenta,red)] [&_[data-slot=slider-range]]:bg-transparent" />
			</div>
		{/if}

		{#if showInputs || showEyeDropper}
			<div class="flex items-center gap-2">
				{#if showInputs}
					<Input
						value={hexDraft}
						oninput={(e) => onHexInput(e.currentTarget.value)}
						class="h-9 font-mono text-sm"
						placeholder="#000000" />
				{/if}
				{#if showEyeDropper && hasEyeDropper}
					<Button
						variant="outline"
						size="icon"
						class="size-9 shrink-0"
						onclick={useEyeDropper}
						title="Pick from screen">
						<PipetteIcon class="size-4" />
					</Button>
				{/if}
			</div>
		{/if}

		{#if showPresets && presets.length > 0}
			<div class="flex flex-wrap gap-1.5">
				{#each presets as preset (preset)}
					<button
						type="button"
						class={cn(
							'size-6 rounded-md border transition-transform hover:scale-110',
							normalizeHex(preset) === previewHex
								? 'ring-2 ring-primary ring-offset-1 ring-offset-popover'
								: '',
						)}
						style="background-color: {preset}"
						title={preset}
						onclick={() => onHexInput(preset)}></button>
				{/each}
			</div>
		{/if}
	</Popover.Content>
</Popover.Root>
