import tailwindcss from '@tailwindcss/vite';
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vitest/config';

export default defineConfig({
	resolve: {
		conditions: ['browser'],
	},
	plugins: [
		tailwindcss(),
		sveltekit({
			compilerOptions: {
				// Force runes mode for the project, except for libraries. Can be removed in svelte 6.
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true,
			},

			adapter: adapter({
				fallback: '404.html',
				// Emit .br/.gz next to each asset; the API serves them via ServeDir::precompressed_*.
				precompress: true,
			}),
		}),
	],
	test: {
		environment: 'jsdom',
		include: ['src/**/*.test.ts'],
	},
});
