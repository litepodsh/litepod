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

			// Enforced through a <meta http-equiv> in each page (prerendered and the 404.html
			// SPA shell): SvelteKit adds the hash of its inline hydration script on every build.
			csp: {
				mode: 'hash',
				directives: {
					// Turnstile loads its script from Cloudflare.
					'script-src': ['self', 'https://challenges.cloudflare.com'],
					// Monaco's language workers.
					'worker-src': ['self', 'blob:'],
					'object-src': ['none'],
					'base-uri': ['self'],
				},
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
