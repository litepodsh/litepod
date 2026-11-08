<script lang="ts">
	import { page } from '$app/state';
	import { Button } from '#lib/components/ui/button/index.js';
	import { t } from '#lib/i18n/index.svelte.js';

	// 3x5 pixel digits, drawn on the same cell board as the app error page Caddy serves.
	const GLYPHS: Record<string, string[]> = {
		'0': ['111', '101', '101', '101', '111'],
		'1': ['010', '110', '010', '010', '111'],
		'2': ['111', '001', '111', '100', '111'],
		'3': ['111', '001', '111', '001', '111'],
		'4': ['101', '101', '111', '001', '001'],
		'5': ['111', '100', '111', '001', '111'],
		'6': ['111', '100', '111', '101', '111'],
		'7': ['111', '001', '010', '010', '010'],
		'8': ['111', '101', '111', '101', '111'],
		'9': ['111', '101', '111', '001', '111']
	};
	// Each glyph pixel is 2x2 cells so the board keeps its fine grain.
	const PX = 2;
	const COLS = 48;
	const ROWS = 18;
	const SIZE = 10;
	const VARIANTS = ['digits', 'scan', 'wander', 'assemble'] as const;
	type Variant = (typeof VARIANTS)[number];
	type Paint = (x: number, y: number, color: 'ink' | 'accent', alpha: number) => void;
	type Draw = (t: number, paint: Paint) => void;

	const notFound = $derived(page.status === 404);
	const code = $derived(String(page.status).padStart(3, '0').slice(-3));
	const lit = $derived.by(() => {
		const width = code.length * 3 * PX + (code.length - 1) * PX;
		const x0 = Math.floor((COLS - width) / 2);
		const y0 = Math.floor((ROWS - 5 * PX) / 2);
		const cells: [number, number][] = [];
		[...code].forEach((digit, n) => {
			(GLYPHS[digit] ?? GLYPHS['0']).forEach((row, j) =>
				[...row].forEach((bit, i) => {
					if (bit !== '1') return;
					for (let dy = 0; dy < PX; dy++)
						for (let dx = 0; dx < PX; dx++)
							cells.push([x0 + n * 4 * PX + i * PX + dx, y0 + j * PX + dy]);
				})
			);
		});
		return cells;
	});
	// SvelteKit's stand-in for unexpected errors says nothing useful; keep it out of the footer.
	const message = $derived(
		page.error?.message && page.error.message !== 'Internal Error' ? page.error.message : null
	);
	const time = new Date().toLocaleTimeString();
	const rand = (n: number) => Math.floor(Math.random() * n);
	const easeOut = (p: number) => 1 - (1 - Math.min(1, Math.max(0, p))) ** 3;

	// 1. The code lights up left to right, then holds.
	function digits(cells: [number, number][]): Draw {
		return (t, paint) => {
			for (const [x, y] of cells) paint(x, y, 'accent', easeOut((t - x * 18 - y * 6) / 420));
		};
	}

	// 2. A beam sweeps the board; the code only shows under it.
	function scan(cells: [number, number][]): Draw {
		const PERIOD = 2800;
		const TAIL = 14;
		return (t, paint) => {
			const beam = ((t % PERIOD) / PERIOD) * (COLS + TAIL) - 2;
			for (let y = 0; y < ROWS; y++) {
				for (let x = 0; x < COLS; x++) {
					const d = beam - x;
					if (d >= 0 && d < 4) paint(x, y, 'accent', 0.22 * (1 - d / 4));
				}
				paint(Math.floor(beam), y, 'accent', 0.55);
			}
			for (const [x, y] of cells) {
				const d = beam - x;
				if (d >= 0 && d < TAIL) paint(x, y, 'accent', 1 - d / TAIL);
				else paint(x, y, 'ink', 0.1);
			}
		};
	}

	// 3. A lost cell wanders the board; the digits it brushes glow, then fade.
	function wander(cells: [number, number][]): Draw {
		const DIRS = [
			[1, 0],
			[-1, 0],
			[0, 1],
			[0, -1]
		];
		const digit = new Set(cells.map(([x, y]) => `${x},${y}`));
		const trail: { x: number; y: number; t: number }[] = [];
		const touched = new Map<string, number>();
		let x = 1;
		let y = Math.floor(ROWS / 2);
		let dir = DIRS[0];
		let next = -1;
		return (t, paint) => {
			// After a hidden tab, skip ahead instead of replaying every missed step.
			if (next < 0 || t - next > 1000) next = t;
			while (t >= next) {
				next += 90;
				if (Math.random() < 0.3) dir = DIRS[rand(4)];
				if (x + dir[0] < 0 || x + dir[0] >= COLS || y + dir[1] < 0 || y + dir[1] >= ROWS)
					dir = [-dir[0], -dir[1]];
				x += dir[0];
				y += dir[1];
				trail.push({ x, y, t: next });
				if (digit.has(`${x},${y}`)) touched.set(`${x},${y}`, next);
			}
			while (trail.length && t - trail[0].t > 1600) trail.shift();
			for (const [cx, cy] of cells) {
				paint(cx, cy, 'ink', 0.16);
				const at = touched.get(`${cx},${cy}`);
				if (at !== undefined) paint(cx, cy, 'accent', 1 - (t - at) / 2400);
			}
			for (const step of trail) paint(step.x, step.y, 'accent', 0.85 * (1 - (t - step.t) / 1600));
			paint(x, y, 'accent', 1);
		};
	}

	// 4. Cells fly in from all over to build the code, hold, then crumble and repeat.
	function assemble(cells: [number, number][]): Draw {
		const CYCLE = 6400;
		let cycle = -1;
		let flying: { x: number; y: number; sx: number; sy: number; d: number; d2: number }[] = [];
		return (t, paint) => {
			if (Math.floor(t / CYCLE) !== cycle) {
				cycle = Math.floor(t / CYCLE);
				flying = cells.map(([x, y]) => ({
					x,
					y,
					sx: rand(COLS),
					sy: rand(ROWS),
					d: Math.random() * 900,
					d2: Math.random() * 900
				}));
			}
			const tau = t % CYCLE;
			for (const c of flying) {
				if (tau < c.d) continue;
				const e = easeOut((tau - c.d) / 700);
				let y = c.sy + (c.y - c.sy) * e;
				let alpha = 0.4 + 0.6 * e;
				const fall = tau - 4800 - c.d2;
				if (fall > 0) {
					y = c.y + (fall / 1000) ** 2 * 30;
					alpha = 1 - fall / 700;
				}
				paint(Math.round(c.sx + (c.x - c.sx) * e), Math.round(y), 'accent', alpha);
			}
		};
	}

	let canvas: HTMLCanvasElement;

	$effect(() => {
		const cells = lit;
		const ctx = canvas.getContext('2d');
		if (!ctx) return;
		const dpr = window.devicePixelRatio || 1;
		canvas.width = COLS * SIZE * dpr;
		canvas.height = ROWS * SIZE * dpr;
		ctx.scale(dpr, dpr);
		const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
		// `#digits`, `#scan`, `#wander` or `#assemble` forces one; otherwise it is a surprise.
		const hash = location.hash.slice(1) as Variant;
		const variant: Variant = still
			? 'digits'
			: VARIANTS.includes(hash)
				? hash
				: VARIANTS[rand(VARIANTS.length)];
		const draw = { digits, scan, wander, assemble }[variant](cells);
		const css = getComputedStyle(canvas);
		let colors = { ink: '', accent: '' };
		const paint: Paint = (x, y, color, alpha) => {
			if (alpha <= 0) return;
			ctx.globalAlpha = Math.min(1, alpha);
			ctx.fillStyle = colors[color];
			ctx.fillRect(x * SIZE + 1, y * SIZE + 1, SIZE - 1, SIZE - 1);
		};
		const start = performance.now();
		let frame = requestAnimationFrame(function loop(now) {
			// Read every frame so a theme switch repaints in the new colors.
			colors = {
				ink: css.getPropertyValue('--foreground').trim(),
				accent: css.getPropertyValue('--primary').trim()
			};
			ctx.clearRect(0, 0, COLS * SIZE, ROWS * SIZE);
			for (let y = 0; y < ROWS; y++) for (let x = 0; x < COLS; x++) paint(x, y, 'ink', 0.06);
			draw(still ? 1e9 : now - start, paint);
			frame = requestAnimationFrame(loop);
		});
		return () => cancelAnimationFrame(frame);
	});
</script>

<main
	class="relative mx-auto flex min-h-screen max-w-2xl flex-col items-center justify-center bg-background px-6 py-10 text-center">
	<canvas
		bind:this={canvas}
		width={COLS * SIZE}
		height={ROWS * SIZE}
		class="board w-full max-w-xl"
		aria-hidden="true"></canvas>

	<p
		class="mt-8 inline-flex items-center gap-2.5 font-mono text-xs tracking-[0.08em] text-muted-foreground uppercase">
		<i class="size-2 bg-primary"></i>
		{page.status} · {notFound ? t('auth.not_found.eyebrow_not_found') : t('auth.not_found.eyebrow_error')}
	</p>
	<h1 class="mt-4 text-[clamp(1.9rem,4.6vw,3.25rem)] leading-none font-semibold tracking-[-0.04em]">
		{notFound ? t('auth.not_found.page_not_found') : t('auth.not_found.error')}
	</h1>
	<p class="mx-auto mt-4 max-w-md text-base leading-relaxed text-muted-foreground">
		{notFound ? t('auth.not_found.not_found_lede') : t('auth.not_found.error_lede')}
	</p>

	<div class="mt-6 flex items-center gap-3">
		<Button href="/">{t('auth.not_found.go_home')}</Button>
		{#if !notFound}
			<Button variant="outline" onclick={() => location.reload()}>
				{t('auth.not_found.retry')}
			</Button>
		{/if}
	</div>

	<footer
		class="mt-10 w-full border-t border-border pt-4 font-mono text-xs leading-normal text-muted-foreground">
		<p class="break-all">{page.url.pathname} · {t('auth.not_found.time')} {time}</p>
		{#if message}<p class="mt-1 opacity-80">{message}</p>{/if}
	</footer>
</main>

<style>
	/* Fade the board out at the edges, like the cell field on the app error page. */
	.board {
		mask-image: radial-gradient(closest-side, #000 55%, transparent);
	}
</style>
