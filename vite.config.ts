import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';

/**
 * Since SvelteKit 3 all project config lives here, in the `sveltekit()` plugin — a
 * `svelte.config.js` file is no longer read and now fails the build on purpose.
 * (SvelteKit 3.0.1, `config_file_unsupported`.)
 *
 * Why each setting:
 * - `adapter-static` with `fallback: 'index.html'`: Tauri loads the frontend from disk
 *   inside a WebView, so there is no Node server at runtime. The SPA fallback keeps
 *   client-side routes working when the WebView reloads (which dev HMR does).
 * - `clearScreen: false`: keeps Tauri's CLI output readable instead of wiping it.
 * - fixed dev port + `strictPort`: `tauri.conf.json` names this port in `devUrl`, so a
 *   port clash must fail loudly rather than point the Tauri window at a stranger.
 * - `envPrefix`: expose `TAURI_*` alongside `VITE_*` so platform branching needs no define.
 *
 * @type {import('vite').UserConfig}
 */
export default defineConfig({
	plugins: [
		tailwindcss(),
		sveltekit({
			adapter: adapter({ fallback: 'index.html' })
		})
	],
	clearScreen: false,
	server: {
		port: 1420,
		strictPort: true,
		watch: { ignored: ['**/src-tauri/**'] }
	},
	envPrefix: ['VITE_', 'TAURI_']
});
