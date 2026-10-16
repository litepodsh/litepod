import { expect, it } from 'vitest';
import { buttonVariants } from './button.svelte';

// Raised buttons light the top-left edge and shade the bottom-right one.
const bevelled = (classes: string) =>
	classes.includes('lp-raised') || classes.includes('inset_1px_1px');

it('raised variants carry the diagonal bevel', () => {
	for (const variant of ['default', 'secondary', 'outline'] as const) {
		expect(bevelled(buttonVariants({ variant })), variant).toBe(true);
	}
});

it('ghost and link stay flat', () => {
	expect(bevelled(buttonVariants({ variant: 'ghost' }))).toBe(false);
	expect(bevelled(buttonVariants({ variant: 'link' }))).toBe(false);
});
