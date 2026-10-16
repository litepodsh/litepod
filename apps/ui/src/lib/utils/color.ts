export type Hsv = { h: number; s: number; v: number };
export type Rgb = { r: number; g: number; b: number };

export function isValidHex(hex: string): boolean {
	return /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(hex.trim());
}

export function normalizeHex(hex: string): string {
	let h = hex.trim().replace(/^#/, '');
	if (h.length === 3) {
		h = h
			.split('')
			.map((c) => c + c)
			.join('');
	}
	return `#${h.toLowerCase()}`;
}

export function hexToRgb(hex: string): Rgb {
	const normalized = normalizeHex(isValidHex(hex) ? hex : '#000000');
	const int = parseInt(normalized.slice(1), 16);
	return {
		r: (int >> 16) & 255,
		g: (int >> 8) & 255,
		b: int & 255,
	};
}

export function rgbToHex({ r, g, b }: Rgb): string {
	const toHex = (n: number) =>
		Math.round(Math.min(255, Math.max(0, n)))
			.toString(16)
			.padStart(2, '0');
	return `#${toHex(r)}${toHex(g)}${toHex(b)}`;
}

export function rgbToHsv({ r, g, b }: Rgb): Hsv {
	const rn = r / 255;
	const gn = g / 255;
	const bn = b / 255;
	const max = Math.max(rn, gn, bn);
	const min = Math.min(rn, gn, bn);
	const delta = max - min;

	let h = 0;
	if (delta !== 0) {
		if (max === rn) h = 60 * (((gn - bn) / delta) % 6);
		else if (max === gn) h = 60 * ((bn - rn) / delta + 2);
		else h = 60 * ((rn - gn) / delta + 4);
	}
	if (h < 0) h += 360;

	const s = max === 0 ? 0 : delta / max;
	const v = max;

	return { h, s: s * 100, v: v * 100 };
}

export function hsvToRgb({ h, s, v }: Hsv): Rgb {
	const sn = s / 100;
	const vn = v / 100;
	const c = vn * sn;
	const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
	const m = vn - c;

	let rp = 0;
	let gp = 0;
	let bp = 0;

	if (h < 60) [rp, gp, bp] = [c, x, 0];
	else if (h < 120) [rp, gp, bp] = [x, c, 0];
	else if (h < 180) [rp, gp, bp] = [0, c, x];
	else if (h < 240) [rp, gp, bp] = [0, x, c];
	else if (h < 300) [rp, gp, bp] = [x, 0, c];
	else [rp, gp, bp] = [c, 0, x];

	return {
		r: (rp + m) * 255,
		g: (gp + m) * 255,
		b: (bp + m) * 255,
	};
}

export function hexToHsv(hex: string): Hsv {
	return rgbToHsv(hexToRgb(hex));
}

export function hsvToHex(hsv: Hsv): string {
	return rgbToHex(hsvToRgb(hsv));
}

function relativeLuminance({ r, g, b }: Rgb): number {
	const toLinear = (c: number) => {
		const cs = c / 255;
		return cs <= 0.03928 ? cs / 12.92 : ((cs + 0.055) / 1.055) ** 2.4;
	};
	const [rl, gl, bl] = [toLinear(r), toLinear(g), toLinear(b)];
	return 0.2126 * rl + 0.7152 * gl + 0.0722 * bl;
}

export function contrastColor(hex: string): '#000000' | '#ffffff' {
	if (!isValidHex(hex)) return '#ffffff';
	return relativeLuminance(hexToRgb(hex)) > 0.45 ? '#000000' : '#ffffff';
}
