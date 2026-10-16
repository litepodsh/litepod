/**
 * The one status vocabulary for the app. Every feature that needs to say
 * "this thing is healthy / working / broken" maps its own status enum onto a
 * StatusTone and asks for classes here, so a running container, a delivered
 * notification and a passing healthcheck are the same green everywhere.
 *
 * Tokens live in `src/app.css` (`--lp-ok` / `--lp-warn` / `--lp-info`, plus the
 * pre-existing `--destructive`) and flip with the theme, so never write a raw
 * Tailwind palette class (an `emerald` or `amber` shade) in a component.
 */
export type StatusTone = 'ok' | 'warn' | 'danger' | 'info' | 'neutral';

/** Filled pill / chip: tinted background plus matching text. */
export function toneClass(tone: StatusTone): string {
	switch (tone) {
		case 'ok':
			return 'bg-success/10 text-success';
		case 'warn':
			return 'bg-warning/10 text-warning';
		case 'danger':
			return 'bg-destructive/10 text-destructive';
		case 'info':
			return 'bg-info/10 text-info';
		case 'neutral':
			return 'bg-muted text-muted-foreground';
	}
}

/** Bare status dot. Colour only, no text. */
export function toneDot(tone: StatusTone): string {
	switch (tone) {
		case 'ok':
			return 'bg-success';
		case 'warn':
			return 'bg-warning';
		case 'danger':
			return 'bg-destructive';
		case 'info':
			return 'bg-info';
		case 'neutral':
			return 'bg-muted-foreground/40';
	}
}

/** Full badge: hairline border, tinted fill and matching text. */
export function toneBadge(tone: StatusTone): string {
	switch (tone) {
		case 'ok':
			return 'border-success/30 bg-success/10 text-success';
		case 'warn':
			return 'border-warning/30 bg-warning/10 text-warning';
		case 'danger':
			return 'border-destructive/30 bg-destructive/10 text-destructive';
		case 'info':
			return 'border-info/30 bg-info/10 text-info';
		case 'neutral':
			return 'border-muted-foreground/30 bg-muted text-muted-foreground';
	}
}

/** Outlined badge: hairline border plus matching text, transparent fill. */
export function toneBorder(tone: StatusTone): string {
	switch (tone) {
		case 'ok':
			return 'border-success/40 text-success';
		case 'warn':
			return 'border-warning/40 text-warning';
		case 'danger':
			return 'border-destructive/40 text-destructive';
		case 'info':
			return 'border-info/40 text-info';
		case 'neutral':
			return 'border-border text-muted-foreground';
	}
}

/** Solid fill for gauges and bars, where a 10% tint would be invisible. */
export function toneFill(tone: StatusTone): string {
	switch (tone) {
		case 'ok':
			return 'var(--success)';
		case 'warn':
			return 'var(--warning)';
		case 'danger':
			return 'var(--destructive)';
		case 'info':
			return 'var(--info)';
		case 'neutral':
			return 'var(--muted-foreground)';
	}
}

/**
 * Shared thresholds for "how full is this resource". Used by the container
 * gauges and any future utilisation bar so 90% means the same thing app-wide.
 */
export function utilizationTone(percent: number | null): StatusTone {
	if (percent === null || !Number.isFinite(percent)) return 'neutral';
	if (percent >= 90) return 'danger';
	if (percent >= 75) return 'warn';
	return 'ok';
}
