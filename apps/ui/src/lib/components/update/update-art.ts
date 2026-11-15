// Pixel-grid animations for the update overlay, in the style of the API's
// error and maintenance pages (apps/api/src/deployment/*.html).

export type UpdateVariant = 'assemble' | 'version' | 'apps' | 'card';
export const UPDATE_VARIANTS: UpdateVariant[] = ['assemble', 'version', 'apps', 'card'];

export type ArtFrame = {
	variant: UpdateVariant;
	preparing: boolean;
	versions: { from: string; to: string } | null;
	labels: { current: string; next: string; apps: string; stages: string[] };
};

type Colors = { ink: string; accent: string; ground: string; muted: string };

const SIZE = 10;
const GAP = 1;

// 3x5 pixel font (the dot is one column wide).
const FONT: Record<string, string> = {
	0: '111101101101111',
	1: '010110010010111',
	2: '111001111100111',
	3: '111001111001111',
	4: '101101111001001',
	5: '111100111001111',
	6: '111100111101111',
	7: '111001001001001',
	8: '111101111101111',
	9: '111101111001111',
	'.': '00001',
	v: '000000101101010',
};

function glyphs(text: string, scale: number) {
	const cells: { i: number; j: number }[] = [];
	let x = 0;
	for (const ch of text) {
		const g = FONT[ch];
		if (!g) continue;
		const w = g.length / 5;
		for (let r = 0; r < 5; r++)
			for (let c = 0; c < w; c++)
				if (g[r * w + c] === '1')
					for (let a = 0; a < scale; a++)
						for (let b = 0; b < scale; b++) cells.push({ i: (x + c) * scale + a, j: r * scale + b });
		x += w + 1;
	}
	return { cells, w: Math.max(0, x - 1) * scale, h: 5 * scale };
}

// Same shape as the favicon: rounded square with a tilted capsule.
function logoCells(n: number) {
	const out: { i: number; j: number; capsule: boolean }[] = [];
	const cos = Math.cos((24 * Math.PI) / 180);
	const sin = Math.sin((24 * Math.PI) / 180);
	for (let j = 0; j < n; j++)
		for (let i = 0; i < n; i++) {
			const x = ((i + 0.5) / n) * 120;
			const y = ((j + 0.5) / n) * 120;
			const dx = Math.max(36 - x, 0, x - 84);
			const dy = Math.max(36 - y, 0, y - 84);
			if (x < 10 || x > 110 || y < 10 || y > 110 || Math.hypot(dx, dy) > 26) continue;
			const rx = (x - 60) * cos + (y - 60) * sin;
			const ry = -(x - 60) * sin + (y - 60) * cos;
			out.push({ i, j, capsule: Math.hypot(Math.max(Math.abs(rx) - 8, 0), ry) <= 18 });
		}
	return out;
}

const hash = (i: number, j: number) => {
	const s = Math.sin(i * 127.1 + j * 311.7) * 43758.5453;
	return s - Math.floor(s);
};

export function createArt(canvas: HTMLCanvasElement) {
	const ctx = canvas.getContext('2d')!;
	let cols = 0;
	let rows = 0;
	let colors: Colors = { ink: '#fff', accent: '#ff6a1f', ground: '#0a0a0a', muted: '#b0b0b0' };

	function cell(x: number, y: number, color: string, alpha = 1) {
		ctx.globalAlpha = alpha;
		ctx.fillStyle = color;
		ctx.fillRect(x * SIZE + GAP, y * SIZE + GAP, SIZE - GAP, SIZE - GAP);
	}
	function label(text: string, x: number, y: number, color: string, alpha = 1) {
		ctx.globalAlpha = alpha;
		ctx.fillStyle = color;
		ctx.font = '500 12px ui-monospace, "SF Mono", Menlo, monospace';
		ctx.textAlign = 'center';
		ctx.textBaseline = 'top';
		ctx.fillText(text, x * SIZE, y * SIZE);
	}

	function assemble(t: number, preparing: boolean) {
		const n = Math.max(8, Math.min(28, rows - 6, cols - 4));
		const ox = Math.floor((cols - n) / 2);
		const oy = Math.max(1, Math.floor((rows - n) / 2));
		const p = (t % 7000) / 7000;
		for (const c of logoCells(n)) {
			const k = hash(c.i, c.j);
			const color = c.capsule ? colors.ink : colors.accent;
			if (preparing) {
				cell(ox + c.i, oy + c.j, color, (t / 900 + k) % 1 < 0.15 ? 0.8 : 0.12);
				continue;
			}
			const on = p < 0.6 ? k < p / 0.6 : p < 0.8 ? true : k > (p - 0.8) / 0.2;
			cell(ox + c.i, oy + c.j, color, on ? 1 : 0.1);
		}
		for (let s = 0; s < 6; s++) {
			const k = hash(s, Math.floor(t / 700));
			cell(
				ox - 3 + Math.floor(k * (n + 6)),
				oy - 3 + Math.floor(hash(k, s) * (n + 6)),
				colors.accent,
				0.35,
			);
		}
	}

	function version(t: number, frame: ArtFrame) {
		const v = frame.versions;
		if (!v || frame.preparing) return assemble(t, frame.preparing);
		const from = `v${v.from}`;
		const to = `v${v.to}`;
		const sweepOf = (w: number) => ((t % 4200) / 3200) * (w + 6);

		let scale = 4;
		let a = glyphs(from, scale);
		let b = glyphs(to, scale);
		const wide = () => a.w + b.w + 9 * scale;
		while (wide() > cols - 4 && scale > 1) {
			scale--;
			a = glyphs(from, scale);
			b = glyphs(to, scale);
		}

		// Narrow screens: stack the two versions instead of a row.
		if (wide() > cols - 4) {
			while ((a.w > cols - 4 || b.w > cols - 4) && scale > 1) scale--;
			const total = a.h + b.h + 6;
			const ay = Math.max(1, Math.floor((rows - total) / 2));
			const by = ay + a.h + 6;
			a.cells.forEach((c) => cell(Math.floor((cols - a.w) / 2) + c.i, ay + c.j, colors.ink, 0.22));
			label(frame.labels.current, cols / 2, ay + a.h + 1, colors.muted, 0.8);
			const bx = Math.floor((cols - b.w) / 2);
			const sweep = sweepOf(b.w);
			b.cells.forEach((c) => {
				const on = c.i < sweep;
				cell(bx + c.i, by + c.j, on ? colors.accent : colors.ink, on ? 1 : 0.1);
			});
			label(frame.labels.next, cols / 2, by + b.h + 1, colors.muted, 0.8);
			return;
		}

		const gap = 3 * scale;
		const arrowW = 3 * scale;
		const total = a.w + gap + arrowW + gap + b.w;
		const ox = Math.floor((cols - total) / 2);
		const oy = Math.max(2, Math.floor((rows - a.h - 3) / 2));
		a.cells.forEach((c) => cell(ox + c.i, oy + c.j, colors.ink, 0.22));
		const ax = ox + a.w + gap;
		const mid = oy + Math.floor(a.h / 2);
		for (let i = 0; i < arrowW; i++) cell(ax + i, mid, colors.muted, 0.6);
		for (let k = 1; k <= scale; k++) {
			cell(ax + arrowW - 1 - k, mid - k, colors.muted, 0.6);
			cell(ax + arrowW - 1 - k, mid + k, colors.muted, 0.6);
		}
		const bx = ax + arrowW + gap;
		const sweep = sweepOf(b.w);
		b.cells.forEach((c) => {
			const on = c.i < sweep;
			cell(bx + c.i, oy + c.j, on ? colors.accent : colors.ink, on ? 1 : 0.1);
		});
		if (sweep < b.w + 1)
			for (let j = -1; j <= b.h; j++) cell(bx + Math.floor(sweep), oy + j, colors.accent, 0.45);
		label(frame.labels.current, ox + a.w / 2, oy + a.h + 2, colors.muted, 0.8);
		label(frame.labels.next, bx + b.w / 2, oy + b.h + 2, colors.muted, 0.8);
	}

	function apps(t: number, frame: ArtFrame) {
		const stages = frame.labels.stages;
		const span = Math.min(cols - 8, 90);
		const ox = Math.floor((cols - span) / 2);
		const y = Math.max(4, Math.floor(rows * 0.3) - 4);
		const step = span / (stages.length - 1);
		// Labels collide below ~11 cells per stage: alternate them above and below.
		const tight = step < 11;
		for (let x = 0; x <= span; x++) cell(ox + x, y + 1, colors.ink, 0.14);
		const pos = frame.preparing ? 0 : ((t % 6000) / 6000) * span;
		stages.forEach((name, k) => {
			const sx = Math.round(ox + k * step) - 1;
			const reached = !frame.preparing && pos >= k * step - 0.5;
			for (let i = 0; i < 3; i++)
				for (let j = 0; j < 3; j++)
					cell(sx + i, y + j, reached ? colors.accent : colors.ink, reached ? 1 : 0.35);
			label(name, sx + 1.5, tight && k % 2 ? y - 2.5 : y + 4, colors.muted, 0.9);
		});
		for (let k = 0; k < 4; k++) cell(ox + Math.floor(pos) - k, y + 1, colors.accent, 1 - k * 0.25);

		const appsY = y + 12;
		if (appsY + 3 > rows) return;
		const n = Math.max(3, Math.min(16, Math.floor(span / 5)));
		const ax = Math.floor((cols - (n * 5 - 2)) / 2);
		label(frame.labels.apps, cols / 2, appsY - 3, colors.muted, 0.9);
		for (let a = 0; a < n; a++) {
			const glow = 0.6 + 0.4 * Math.sin(t / 900 + a * 0.7);
			for (let i = 0; i < 3; i++)
				for (let j = 0; j < 3; j++) {
					const hole = i === 1 && j === 1;
					cell(ax + a * 5 + i, appsY + j, hole ? colors.ground : colors.ink, hole ? 1 : glow * 0.8);
				}
		}
	}

	return {
		// `height` is the part of the canvas the art may use, in CSS pixels.
		resize(height: number) {
			const dpr = window.devicePixelRatio || 1;
			const { width, height: full } = canvas.getBoundingClientRect();
			canvas.width = width * dpr;
			canvas.height = full * dpr;
			ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
			cols = Math.floor(width / SIZE);
			rows = Math.floor(height / SIZE);
		},
		readColors(from: Element) {
			const css = getComputedStyle(from);
			colors = {
				ink: css.getPropertyValue('--lp-ink').trim(),
				accent: css.getPropertyValue('--lp-accent').trim(),
				ground: css.getPropertyValue('--lp-ground').trim(),
				muted: css.getPropertyValue('--lp-muted').trim(),
			};
		},
		draw(t: number, frame: ArtFrame) {
			ctx.clearRect(0, 0, canvas.width, canvas.height);
			if (frame.variant === 'version') version(t, frame);
			else if (frame.variant === 'apps') apps(t, frame);
			else assemble(t, frame.preparing);
			ctx.globalAlpha = 1;
		},
	};
}
